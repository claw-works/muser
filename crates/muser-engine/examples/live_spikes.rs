//! 现场模式下逐块渲染整首，列出最慢的几块（掉帧的嫌疑）：`cargo run --release -p muser-engine --example live_spikes -- <工程>`。
fn main() {
    let path = std::env::args().nth(1).expect("工程文件");
    let p = muser_core::Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut r = muser_engine::Renderer::live(&p, muser_engine::RenderSettings::default()).unwrap();
    let map = p.bar_map();
    let mut block = manguebyte::prelude::AudioBuffer::<f32>::new(2, 256);
    let mut times = Vec::new();
    let mut at = 0usize;
    loop {
        let t = std::time::Instant::now();
        let n = r.process(&mut block).unwrap();
        if n == 0 {
            break;
        }
        times.push((t.elapsed(), at));
        at += n;
    }
    let total: std::time::Duration = times.iter().map(|t| t.0).sum();
    println!("{} 块，共 {:?}，音频 {:.1} 秒（实时的 {:.1}%）", times.len(), total, at as f64 / 48_000.0, total.as_secs_f64() / (at as f64 / 48_000.0) * 100.0);
    times.sort_by_key(|t| std::cmp::Reverse(t.0));
    for (d, f) in times.iter().take(12) {
        let beat = *f as f64 / 48_000.0 * p.tempo / 60.0;
        println!("{d:>10.2?}  第 {} 小节（{:.1} 秒）", map.bar_at(beat).0 + 1, *f as f64 / 48_000.0);
    }
}
