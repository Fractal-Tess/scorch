//! Discriminating probe for where Obscura's script time actually goes.
//!
//! Obscura runs V8 (via deno_core), the same engine Chromium uses, so slow
//! script execution cannot be blamed on the JIT. This times pure-computation JS
//! against DOM-touching JS in the same isolate, then measures how DOM lookups
//! scale with tree size.
//!
//! The scaling pass is the decisive one: a browser with the usual id/class/tag
//! indices answers `querySelector` in roughly constant time, and caches layout
//! behind a dirty bit so repeated `getBoundingClientRect` reads are nearly free.
//! Cost that grows linearly with node count means no index and no layout cache.
//!
//! Run with: cargo run --release -p scorch-engine --example js_probe

use std::{sync::Arc, time::Instant};

use obscura_browser::{BrowserContext, Page};

/// Non-mutating cases, safe to run against a shared pristine DOM.
const CASES: &[(&str, u64, &str)] = &[
    (
        "compute: arithmetic",
        5_000_000,
        "let s=0; for(let i=0;i<N;i++){ s+=i*2^(i&7); } return s;",
    ),
    (
        "compute: array push/read",
        1_000_000,
        "const a=[]; for(let i=0;i<N;i++){ a.push(i); } let s=0; \
         for(let i=0;i<N;i++){ s+=a[i]; } return s;",
    ),
    (
        "dom: createElement (detached)",
        20_000,
        "let n=0; for(let i=0;i<N;i++){ const e=document.createElement('div'); \
         n+=e?1:0; } return n;",
    ),
    (
        "dom: get/setAttribute (detached)",
        20_000,
        "const e=document.createElement('div'); let n=0; \
         for(let i=0;i<N;i++){ e.setAttribute('data-x', i); \
         n+=e.getAttribute('data-x')?1:0; } return n;",
    ),
    (
        "dom: textContent write (detached)",
        20_000,
        "const e=document.createElement('div'); let n=0; \
         for(let i=0;i<N;i++){ e.textContent='v'+i; \
         n+=e.textContent.length?1:0; } return n;",
    ),
];

/// Lookups timed against a controlled tree size, to expose scaling behaviour.
const SCALING: &[(&str, &str)] = &[
    (
        "querySelector('#target')",
        "let n=0; for(let i=0;i<N;i++){ n+=document.querySelector('#target')?1:0; } return n;",
    ),
    (
        "getElementById('target')",
        "let n=0; for(let i=0;i<N;i++){ n+=document.getElementById('target')?1:0; } return n;",
    ),
    (
        "getElementsByTagName('span')",
        "let n=0; for(let i=0;i<N;i++){ \
         n+=document.getElementsByTagName('span').length?1:0; } return n;",
    ),
    (
        "getBoundingClientRect()",
        "let n=0; for(let i=0;i<N;i++){ \
         n+=document.body.getBoundingClientRect().width>=0?1:0; } return n;",
    ),
];

const SIZES: &[u64] = &[100, 1_000, 10_000];

fn time_ns(page: &mut Page, script: &str, iterations: u64) -> Option<f64> {
    let wrapped = format!("(function(){{ const N={iterations}; {script} }})()");
    let _ = page.evaluate(&wrapped); // warm the JIT and any lazy op setup
    let started = Instant::now();
    let value = page.evaluate(&wrapped);
    let elapsed = started.elapsed();
    if value.is_null() {
        None
    } else {
        Some(elapsed.as_nanos() as f64 / iterations as f64)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let context = Arc::new(BrowserContext::with_options(
        "js-probe".to_owned(),
        None,
        false,
    ));
    let mut page = Page::new("probe".to_owned(), context);
    page.set_viewport((1280.0, 720.0));
    if let Err(error) = page.navigate("https://example.com").await {
        eprintln!("navigate failed: {error}; is the network reachable?");
        return;
    }

    println!("== per-operation cost (pristine example.com DOM) ==");
    println!("{:<36} {:>10} {:>14}", "case", "ops", "ns/op");
    for (label, iterations, body) in CASES {
        match time_ns(&mut page, body, *iterations) {
            Some(ns) => println!("{label:<36} {iterations:>10} {ns:>14.1}"),
            None => println!("{label:<36} {iterations:>10} {:>14}", "FAILED"),
        }
    }

    println!("\n== DOM lookup cost vs tree size (ns per call) ==");
    print!("{:<32}", "operation");
    for size in SIZES {
        print!("{:>14}", format!("{size} nodes"));
    }
    println!("{:>12}", "growth");

    let mut rows: Vec<(String, Vec<Option<f64>>)> = Vec::new();
    for (label, script) in SCALING {
        rows.push((label.to_string(), Vec::new()));
        let _ = label;
        let _ = script;
    }

    for (index, size) in SIZES.iter().enumerate() {
        // Rebuild the tree from scratch at this size so earlier cases cannot
        // leave nodes behind and inflate the next measurement.
        let build = format!(
            "(function(){{ document.body.innerHTML=''; \
             const f=document.createDocumentFragment(); \
             for(let i=0;i<{size};i++){{ const e=document.createElement('span'); \
             e.className='c'+(i%10); f.appendChild(e); }} \
             const t=document.createElement('div'); t.id='target'; f.appendChild(t); \
             document.body.appendChild(f); \
             return document.getElementsByTagName('span').length; }})()"
        );
        let built = page.evaluate(&build);
        if built.is_null() {
            eprintln!("failed to build a {size}-node tree");
            return;
        }
        for (row, (_, script)) in rows.iter_mut().zip(SCALING) {
            let iterations = if *size >= 10_000 { 200 } else { 2_000 };
            row.1.push(time_ns(&mut page, script, iterations));
            let _ = index;
        }
    }

    for (label, samples) in &rows {
        print!("{label:<32}");
        for sample in samples {
            match sample {
                Some(ns) => print!("{ns:>14.0}"),
                None => print!("{:>14}", "FAILED"),
            }
        }
        match (
            samples.first().and_then(|s| *s),
            samples.last().and_then(|s| *s),
        ) {
            (Some(first), Some(last)) if first > 0.0 => println!("{:>11.0}x", last / first),
            _ => println!("{:>12}", "-"),
        }
    }
    println!("\n(100x growth in tree size; ~1x cost = indexed, ~100x = linear scan)");
}
