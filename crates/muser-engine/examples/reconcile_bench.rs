//! 量现场模式换工程（reconcile）要多久：`cargo run --release -p muser-engine --example reconcile_bench -- <工程>`。
fn main() {
    let path = std::env::args().nth(1).expect("工程文件");
    let p = muser_core::Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let settings = muser_engine::RenderSettings::default();
    let t = std::time::Instant::now();
    let mut r = muser_engine::Renderer::live(&p, settings).unwrap();
    println!("建渲染器 {:?}", t.elapsed());
    let mut block = manguebyte::prelude::AudioBuffer::<f32>::new(2, 48_000);
    for _ in 0..3 {
        r.process(&mut block).unwrap();
    }
    for _ in 0..3 {
        let t = std::time::Instant::now();
        r.reconcile(&p).unwrap();
        println!("对账 {:?}（{} 拍）", t.elapsed(), p.duration_beats().0);
    }
}
