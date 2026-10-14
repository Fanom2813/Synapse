use synapse_app::perf_probe::{run_probe, ProbeArgs};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(args) = ProbeArgs::parse(&argv) {
        let report = run_probe(args.scenario, args.seconds);
        println!("{}", report.to_json());
        if !report.passed {
            std::process::exit(1);
        }
        return;
    }
    println!("synapse: app shell lands in M1 task 2 (see docs/MILESTONES.md).");
}
