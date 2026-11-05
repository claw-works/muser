//! 量建一件乐器要多久（第一次与第二次）：`cargo run --release -p muser-engine --example instrument_build_bench -- strings.violin …`。
fn main() {
    let spec = manguebyte::prelude::ProcessSpec { sample_rate: 48_000.0, max_block_size: 512, num_channels: 2 };
    for name in std::env::args().skip(1) {
        for k in 0..2 {
            let t = std::time::Instant::now();
            let mut i = muser_engine::instrument::resolve_for(&name, &Default::default(), &spec).unwrap();
            let built = t.elapsed();
            i.prepare(&spec);
            println!("{name} 第 {} 次：建 {built:?}，prepare 后共 {:?}", k + 1, t.elapsed());
        }
    }
}
