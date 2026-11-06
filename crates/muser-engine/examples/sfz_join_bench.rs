//! 量现场演奏中途加一条 SFZ 轨时换工程（reconcile）要多久、两台 SFZ 钢琴一起弹每块渲染多久：
//! `cargo run --release -p muser-engine --example sfz_join_bench -- <sfz 文件> [第二个 sfz 文件]`。
use std::time::Instant;

fn project(sfz: &[String]) -> muser_core::Project {
    let mut tracks = vec![serde_json::json!({ "name": "pad", "instrument": "poly.warm_analog_pad",
        "clips": [{ "start": 0, "length": 64, "notes": [{ "pitch": 57, "start": 0, "length": 64 }] }] })];
    for (i, s) in sfz.iter().enumerate() {
        // 8 小节、每拍一个三音和弦加一串十六分音符：复音多、尾音长。
        let mut notes = Vec::new();
        for b in 8..64 {
            for p in [48, 55, 64] { notes.push(serde_json::json!({ "pitch": p + i as u8 * 12, "start": b, "length": 1.0 })); }
            for k in 0..4 { notes.push(serde_json::json!({ "pitch": 72 + (k * 3) as u8, "start": b as f64 + k as f64 * 0.25, "length": 0.25 })); }
        }
        tracks.push(serde_json::json!({ "name": format!("piano{i}"), "instrument": format!("sfz:{s}"), "clips": [{ "start": 0, "length": 64, "notes": notes }] }));
    }
    serde_json::from_value(serde_json::json!({ "tempo": 120, "tracks": tracks })).unwrap()
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let settings = muser_engine::RenderSettings::default();
    let mut r = muser_engine::Renderer::live(&project(&[]), settings).unwrap();
    let mut block = manguebyte::prelude::AudioBuffer::<f32>::new(2, 512);
    for _ in 0..50 { r.process(&mut block).unwrap(); }
    for n in 1..=files.len() {
        let p = project(&files[..n]);
        // 现场演奏里提交时先在别的线程加载（muser live 的做法）；拿着它，播放线程里的对账就不再读采样。
        let t = Instant::now();
        let held = std::thread::scope(|sc| sc.spawn(|| muser_engine::instrument::preload_sfz(&p).unwrap()).join().unwrap());
        println!("加第 {n} 台：后台预加载 {:?}", t.elapsed());
        let t = Instant::now();
        r.reconcile(&p).unwrap();
        println!("加第 {n} 台：对账 {:?}", t.elapsed());
        let t = Instant::now();
        r.reconcile(&p).unwrap();
        println!("  再对账一次（已经加载过）{:?}", t.elapsed());
        // 弹起来以后：每 512 帧一块（10.7 ms）渲染多久。
        let (mut worst, mut total, blocks) = (0f64, 0f64, 400);
        for _ in 0..blocks {
            let t = Instant::now();
            r.process(&mut block).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            worst = worst.max(ms);
            total += ms;
        }
        drop(held);
        println!("  每块平均 {:.2} ms，最慢 {:.2} ms（实时预算 10.67 ms）", total / blocks as f64, worst);
    }
}
