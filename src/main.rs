mod core;

use std::collections::BTreeMap;

use core::aggregate::{self, Totals};
use core::sources::{self, Config};

fn main() -> anyhow::Result<()> {
    let smoke = std::env::args().any(|arg| arg == "--smoke");
    if smoke {
        return smoke_main();
    }
    ui::run()
}

/// Print the whole ledger to stdout: the cross-GUI merge, proven on real data.
fn smoke_main() -> anyhow::Result<()> {
    let config = Config::load();
    let billing = config.billing();
    let (records, statuses) = sources::scan_all(&config);
    println!("== sources ==");
    for status in &statuses {
        println!(
            "{:>10}  {:>7} records  {}",
            status.label, status.records, status.detail
        );
    }
    println!("\n== model × source (the merge) ==");
    let mut table: BTreeMap<String, BTreeMap<String, Totals>> = BTreeMap::new();
    for rec in &records {
        table
            .entry(rec.model_key.clone())
            .or_default()
            .entry(rec.source.clone())
            .or_default()
            .add(rec, &billing);
    }
    for (model, by_source) in &table {
        let mut all = Totals::default();
        for (source, totals) in by_source {
            let _ = all.requests; // silence unused in loop; real sum below
            all.requests += totals.requests;
            all.input_net += totals.input_net;
            all.cache_read += totals.cache_read;
            all.output += totals.output;
            all.credits += totals.credits;
            let _ = source;
        }
        println!(
            "{:<28} {:>8} req {:>12} tok {:>10.1} cr  [{}]",
            model,
            all.requests,
            all.input_net + all.cache_read + all.output,
            all.credits,
            by_source
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(" + ")
        );
    }
    let total: Totals = records.iter().fold(Totals::default(), |mut sum, rec| {
        sum.add(rec, &billing);
        sum
    });
    println!(
        "\n== total == {} requests, {} tokens, {:.1} credits, cache hit {:.1}%",
        total.requests,
        total.input_net + total.cache_read + total.output,
        total.credits,
        total.cache_hit_rate().unwrap_or(0.0) * 100.0
    );
    let days = aggregate::Daily::build(
        &records,
        |rec| rec.source.clone(),
        "credits",
        &billing,
    );
    println!("days: {}", days.days.len());
    Ok(())
}

mod ui;
