#[macro_use]
extern crate serde_derive;
extern crate chan_signal;
extern crate docopt;
extern crate rand;
extern crate sled;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use chan_signal::Signal;
use docopt::Docopt;
use rand::{thread_rng, Rng};

const USAGE: &'static str = "
Usage: stress [--threads=<#>] [--burn-in] [--duration=<s>]

Options:
    --threads=<#>      Number of threads [default: 4].
    --burn-in          Don't halt until we receive a signal.
    --duration=<s>     Seconds to run for [default: 10].
    --kv-len=<l>       The length of both keys and values [default: 2].
";

#[derive(Deserialize)]
struct Args {
    flag_threads: usize,
    flag_burn_in: bool,
    flag_duration: u64,
    flag_kv_len: usize,
}

fn report(shutdown: Arc<AtomicBool>, total: Arc<AtomicUsize>) {
    let mut last = 0;
    while !shutdown.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_secs(1));
        let total = total.load(Ordering::Acquire);

        println!("did {} ops", total - last);

        last = total;
    }
}

fn run(
    tree: Arc<sled::Tree>,
    shutdown: Arc<AtomicBool>,
    total: Arc<AtomicUsize>,
    kv_len: usize,
) {
    let byte = || {
        thread_rng()
            .gen_iter::<u8>()
            .take(kv_len)
            .collect::<Vec<_>>()
    };
    let mut rng = thread_rng();

    while !shutdown.load(Ordering::Relaxed) {
        total.fetch_add(1, Ordering::Release);
        let choice = rng.gen_range(0, 5);

        match choice {
            0 => {
                tree.set(byte(), byte()).unwrap();
            }
            1 => {
                tree.get(&*byte()).unwrap();
            }
            2 => {
                tree.del(&*byte()).unwrap();
            }
            3 => match tree.cas(byte(), Some(byte()), Some(byte())) {
                Ok(_) | Err(sled::Error::CasFailed(_)) => {}
                other => panic!("operational error: {:?}", other),
            },
            4 => {
                let _ = tree
                    .scan(&*byte())
                    .take(rng.gen_range(0, 15))
                    .map(|res| res.unwrap())
                    .collect::<Vec<_>>();
            }
            _ => panic!("impossible choice"),
        }
    }
}

fn main() {
    let signal = chan_signal::notify(&[Signal::INT, Signal::TERM]);

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| {
            d.argv(std::env::args().into_iter()).deserialize()
        }).unwrap_or_else(|e| e.exit());

    let total = Arc::new(AtomicUsize::new(0));
    let shutdown = Arc::new(AtomicBool::new(false));

    let config = sled::ConfigBuilder::new()
        .io_bufs(2)
        .io_buf_size(8_000_000)
        .blink_fanout(32)
        .page_consolidation_threshold(10)
        .cache_bits(6)
        .cache_capacity(1_000_000)
        .flush_every_ms(Some(100))
        .snapshot_after_ops(1000000)
        .print_profile_on_drop(true)
        .build();

    let tree = Arc::new(sled::Tree::start(config).unwrap());

    let mut threads = vec![];

    let now = std::time::Instant::now();

    let n_threads = args.flag_threads;
    let kv_len = args.flag_kv_len;

    for i in 0..n_threads + 1 {
        let tree = tree.clone();
        let shutdown = shutdown.clone();
        let total = total.clone();

        let t = if i == 0 {
            thread::spawn(move || report(shutdown, total))
        } else {
            thread::spawn(move || run(tree, shutdown, total, kv_len))
        };

        threads.push(t);
    }

    if args.flag_burn_in {
        println!("waiting on signal");
        signal.recv();
        println!("got shutdown signal, cleaning up...");
    } else {
        thread::sleep(std::time::Duration::from_secs(
            args.flag_duration,
        ));
    }

    shutdown.store(true, Ordering::SeqCst);

    for t in threads.into_iter() {
        t.join().unwrap();
    }

    let ops = total.load(Ordering::SeqCst);
    let time = now.elapsed().as_secs() as usize;

    println!(
        "did {} total ops in {} seconds. {} ops/s",
        ops,
        time,
        (ops * 1_000) / (time * 1_000)
    );
}
