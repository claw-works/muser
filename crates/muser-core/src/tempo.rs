//! 速度表：拍 ↔ 秒。工程有一个起始速度（`tempo`），段落可以换速度（`sections.<名>.tempo`），
//! 可选在段落开头的 `tempo_ramp_bars` 小节内从上一段的速度渐变过来（不写或 0 = 在段落开头直接切换）。
//!
//! 渐变按**时间**线性（每一秒加同样多的 BPM），和 manguebyte 的 `TempoCurve::Linear`、CLAP 的 `tempo_inc` 一样：
//! 插件走带看到的速度与 muser 排音符用的速度一致。速度不变时的换算和原来的 `拍 × 60 / BPM` 逐位相同。

/// 一段：从第 `beat` 拍（第 `secs` 秒）开始，速度 `bpm`；`ramp` 是渐变：(这一段结束的拍, 结束时的 BPM)，
/// 期间按时间线性变化；`None` 是不变。最后一段一直延续下去。
#[derive(Debug, Clone, PartialEq)]
struct Piece {
    beat: f64,
    secs: f64,
    bpm: f64,
    ramp: Option<(f64, f64)>,
}

impl Piece {
    /// 这一段里过 `db` 拍要多少秒。
    fn secs_for(&self, db: f64) -> f64 {
        match self.ramp {
            None => db * (60.0 / self.bpm),
            Some((end, b1)) => {
                // bpm(t) = a + k·t，拍(t) = (a·t + k·t²/2) / 60；整段 T = 120·L / (a + b)。
                let (a, len) = (self.bpm, end - self.beat);
                let total = 120.0 * len / (a + b1);
                let k = (b1 - a) / total;
                if k.abs() < 1e-12 {
                    return db * (60.0 / a);
                }
                (-a + (a * a + 2.0 * k * 60.0 * db).max(0.0).sqrt()) / k
            }
        }
    }

    /// 这一段里过 `dt` 秒走了多少拍。
    fn beats_for(&self, dt: f64) -> f64 {
        match self.ramp {
            None => dt * self.bpm / 60.0,
            Some((end, b1)) => {
                let (a, len) = (self.bpm, end - self.beat);
                let k = (b1 - a) / (120.0 * len / (a + b1));
                (a * dt + k * dt * dt / 2.0) / 60.0
            }
        }
    }

    fn end_secs(&self) -> f64 {
        self.ramp.map_or(f64::INFINITY, |(end, _)| self.secs + self.secs_for(end - self.beat))
    }
}

/// 速度表。见模块说明。
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    pieces: Vec<Piece>,
}

impl TempoMap {
    /// 一个速度到底。
    pub fn constant(bpm: f64) -> TempoMap {
        TempoMap { pieces: vec![Piece { beat: 0.0, secs: 0.0, bpm, ramp: None }] }
    }

    /// 从第 `beat` 拍起换成 `bpm`；`ramp_beats > 0` 时在这么多拍内从当时的速度渐变到 `bpm`。按拍的先后调用。
    pub fn change(&mut self, beat: f64, bpm: f64, ramp_beats: f64) {
        let from = self.bpm_at(beat);
        // 去掉 beat 及之后的（按顺序调用时只会是同一拍上的）。
        self.pieces.retain(|p| p.beat < beat - 1e-9);
        if self.pieces.is_empty() {
            self.pieces.push(Piece { beat: 0.0, secs: 0.0, bpm: from, ramp: None });
        }
        // 前一段（可能是渐变）截在 beat。
        let last = self.pieces.last_mut().expect("至少一段");
        if let Some((end, _)) = last.ramp {
            if end > beat {
                // 渐变还没走完就换：保持当时的速度为止点（极少见，按直线截断）。
                let b_at = from;
                last.ramp = Some((beat, b_at));
            }
        }
        let secs = self.secs_at(beat);
        if ramp_beats > 1e-9 && (bpm - from).abs() > 1e-9 {
            self.pieces.push(Piece { beat, secs, bpm: from, ramp: Some((beat + ramp_beats, bpm)) });
            let s2 = self.secs_at(beat + ramp_beats);
            self.pieces.push(Piece { beat: beat + ramp_beats, secs: s2, bpm, ramp: None });
        } else if (bpm - from).abs() > 1e-9 || beat <= 1e-9 || self.pieces.last().is_some_and(|p| p.ramp.is_some()) {
            if beat <= 1e-9 {
                self.pieces = vec![Piece { beat: 0.0, secs: 0.0, bpm, ramp: None }];
            } else {
                self.pieces.push(Piece { beat, secs, bpm, ramp: None });
            }
        }
    }

    /// 速度一直不变。
    pub fn is_constant(&self) -> bool {
        self.pieces.len() == 1 && self.pieces[0].ramp.is_none()
    }

    fn piece_at_beat(&self, beat: f64) -> &Piece {
        let i = self.pieces.partition_point(|p| p.beat <= beat).saturating_sub(1);
        &self.pieces[i]
    }

    fn piece_at_secs(&self, secs: f64) -> &Piece {
        let i = self.pieces.partition_point(|p| p.secs <= secs).saturating_sub(1);
        &self.pieces[i]
    }

    /// 第 `beat` 拍在第几秒。
    pub fn secs_at(&self, beat: f64) -> f64 {
        if self.pieces.len() == 1 && self.pieces[0].ramp.is_none() {
            return beat * (60.0 / self.pieces[0].bpm);
        }
        let p = self.piece_at_beat(beat);
        p.secs + p.secs_for(beat - p.beat)
    }

    /// 从第 `base` 拍到第 `base + delta` 拍有多少秒（速度不变时就是 `delta × 60 / BPM`，与原来的算法逐位相同）。
    pub fn secs_between(&self, base: f64, delta: f64) -> f64 {
        if self.pieces.len() == 1 && self.pieces[0].ramp.is_none() {
            return delta * (60.0 / self.pieces[0].bpm);
        }
        self.secs_at(base + delta) - self.secs_at(base)
    }

    /// 第 `secs` 秒是第几拍。
    pub fn beat_at(&self, secs: f64) -> f64 {
        if self.pieces.len() == 1 && self.pieces[0].ramp.is_none() {
            return secs * self.pieces[0].bpm / 60.0;
        }
        let p = self.piece_at_secs(secs);
        let dt = secs - p.secs;
        match p.ramp {
            Some((end, _)) if secs >= p.end_secs() => end + (secs - p.end_secs()) * self.bpm_at(end) / 60.0,
            _ => p.beat + p.beats_for(dt),
        }
    }

    /// 第 `beat` 拍时的速度（BPM）。
    pub fn bpm_at(&self, beat: f64) -> f64 {
        let p = self.piece_at_beat(beat);
        match p.ramp {
            None => p.bpm,
            Some((end, b1)) if beat >= end => b1,
            Some((end, b1)) => {
                // 按时间线性：先换算成这一段里的秒，再插值。
                let t = p.secs_for(beat - p.beat);
                let total = p.secs_for(end - p.beat);
                p.bpm + (b1 - p.bpm) * (t / total).clamp(0.0, 1.0)
            }
        }
    }

    /// 每一个换速度的点：(拍, BPM, 到下一个点是不是按时间线性渐变)。给插件走带、MIDI 导出用。
    pub fn points(&self) -> Vec<(f64, f64, bool)> {
        self.pieces.iter().map(|p| (p.beat, p.bpm, p.ramp.is_some())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 渐变还没走完就换成同一个速度：截在那里，之后保持当时的速度。
    #[test]
    fn a_ramp_cut_short_holds_the_tempo_it_reached() {
        let mut m = TempoMap::constant(60.0);
        m.change(0.0, 60.0, 0.0);
        m.change(4.0, 120.0, 8.0);
        let mid = m.bpm_at(8.0);
        m.change(8.0, mid, 0.0);
        assert!((m.bpm_at(20.0) - mid).abs() < 1e-9, "{} vs {mid}", m.bpm_at(20.0));
        assert!((m.secs_between(10.0, 6.0) - 6.0 * 60.0 / mid).abs() < 1e-9);
    }

    #[test]
    fn constant_matches_the_old_formula() {
        let m = TempoMap::constant(97.3);
        for b in [0.0, 1.0, 3.75, 1234.5] {
            assert_eq!(m.secs_at(b), b * (60.0 / 97.3));
            assert_eq!(m.secs_between(10.0, b), b * (60.0 / 97.3));
        }
    }

    #[test]
    fn steps_and_ramps() {
        let mut m = TempoMap::constant(120.0);
        m.change(8.0, 60.0, 0.0); // 第 8 拍起 60 BPM
        assert!((m.secs_at(8.0) - 4.0).abs() < 1e-12);
        assert!((m.secs_at(10.0) - 6.0).abs() < 1e-12);
        assert!((m.beat_at(6.0) - 10.0).abs() < 1e-9);
        m.change(16.0, 120.0, 8.0); // 第 16 拍起 8 拍内从 60 渐变到 120
                                    // 按时间线性：8 拍用 120·8/(60+120) = 5.333 秒。
        assert!((m.secs_at(24.0) - m.secs_at(16.0) - 120.0 * 8.0 / 180.0).abs() < 1e-9);
        assert!((m.bpm_at(16.0) - 60.0).abs() < 1e-9 && (m.bpm_at(24.0) - 120.0).abs() < 1e-9);
        let mid = m.bpm_at(20.0);
        assert!(mid > 60.0 && mid < 120.0, "{mid}");
        // 来回换算一致。
        for b in [0.0, 5.0, 8.0, 12.3, 16.0, 17.5, 23.9, 24.0, 40.0] {
            assert!((m.beat_at(m.secs_at(b)) - b).abs() < 1e-9, "{b}");
        }
        // 速度单调变化：秒随拍单调增。
        let mut last = -1.0;
        for i in 0..400 {
            let s = m.secs_at(i as f64 * 0.1);
            assert!(s > last);
            last = s;
        }
        assert_eq!(m.points().len(), 4);
    }
}
