use crossbeam::channel;
use std::thread;

pub fn run_pipeline(block_count: usize, block_size: usize) -> usize {
    let (tx, rx) = channel::bounded::<Vec<u8>>(4);

    let producer = thread::spawn(move || {
        for _ in 0..block_count {
            let block = vec![0u8; block_size];
            if tx.send(block).is_err() {
                return;
            }
        }
    });

    let consumer = thread::spawn(move || {
        let mut processed = 0usize;
        while let Ok(block) = rx.recv() {
            let _ = block.len();
            processed += 1;
        }
        processed
    });

    let _ = producer.join();
    consumer.join().unwrap_or(0)
}
