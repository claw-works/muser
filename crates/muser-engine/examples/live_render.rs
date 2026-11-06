//! 用现场模式（Renderer::live）渲染整首，报每 5 秒的峰值：`cargo run --release -p muser-engine --features host --example live_render -- <工程>`。
fn main() {
    let path = std::env::args().nth(1).expect("工程文件");
    let p = muser_core::Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut r = muser_engine::Renderer::live(&p, muser_engine::RenderSettings::default()).unwrap();
    let mut block = manguebyte::prelude::AudioBuffer::<f32>::new(2, 48_000 * 5);
    let mut t = 0;
    while let n @ 1.. = r.process(&mut block).unwrap() {
        let peak = block.channel(0)[..n].iter().fold(0f32, |m, x| m.max(x.abs()));
        println!("{:>4}–{:>4} 秒  峰值 {:.4}", t, t + 5, peak);
        t += 5;
    }
}
