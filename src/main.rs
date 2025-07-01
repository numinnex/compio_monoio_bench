use std::{
    time::{Duration, Instant},
};

use compio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    BufResult,
};

use monoio::{
    io::{AsyncReadRentExt, AsyncWriteRentExt},
    net::{TcpListener as MonoioTcpListener, TcpStream as MonoioTcpStream},
};

const PACKET_SIZE: usize = 4096; 

#[derive(Debug)]
struct ClientResults {
    latencies: Vec<Duration>,
    throughput_rps: f64,
}

#[derive(Debug)]
struct PerformanceMetrics {
    total_requests: usize,
    total_time: Duration,
    throughput_rps: f64,
    avg_latency: Duration,
    p50_latency: Duration,
    p90_latency: Duration,
    p99_latency: Duration,
    p999_latency: Duration,
}

impl PerformanceMetrics {
    fn new(client_results: ClientResults) -> Self {
        let mut latencies = client_results.latencies;
        let total_requests = latencies.len();
        latencies.sort();
        
        let avg_latency = latencies.iter().sum::<Duration>() / total_requests as u32;
        
        let p50_latency = latencies[total_requests * 50 / 100];
        let p90_latency = latencies[total_requests * 90 / 100];
        let p99_latency = latencies[total_requests * 99 / 100];
        let p999_latency = latencies[total_requests * 999 / 1000];
        
        Self {
            total_requests,
            total_time: Duration::from_secs_f64(total_requests as f64 / client_results.throughput_rps),
            throughput_rps: client_results.throughput_rps,
            avg_latency,
            p50_latency,
            p90_latency,
            p99_latency,
            p999_latency,
        }
    }
    
    fn print_report(&self) {
        println!("=== Performance Report ===");
        println!("total requests: {}", self.total_requests);
        println!("total time: {:?}", self.total_time);
        println!("throughput: {:.2} rps", self.throughput_rps);
        println!("average Latency: {:?}", self.avg_latency);
        println!("p50 Latency: {:?}", self.p50_latency);
        println!("p90 Latency: {:?}", self.p90_latency);
        println!("p99 Latency: {:?}", self.p99_latency);
        println!("p999 Latency: {:?}", self.p999_latency);
    }
}

fn run_compio_server() {
    let proactor = compio::driver::ProactorBuilder::new()
        .coop_taskrun(true)
        .capacity(1024)
        .to_owned();
    let rt = compio::runtime::RuntimeBuilder::new()
        .with_proactor(proactor)
        .build()
        .unwrap();
    rt.block_on(async move {
        let listener = TcpListener::bind("127.0.0.1:2137").await.unwrap();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let shutdown = compio::runtime::spawn(async move {
                loop {
                    let buf = vec![0; PACKET_SIZE];
                    let BufResult(result, buf) = stream.read_exact(buf).await;
                    if result.is_err() {
                        break;
                    } else {
                        stream.write_all(buf).await.unwrap();
                    }
                }
                return true;
            })
            .await
            .unwrap();
            if shutdown {
                break;
            }
        }
    })
}

fn run_compio_client(num_requests: usize) -> ClientResults {
    let mut results = Vec::with_capacity(num_requests);
    let proactor = compio::driver::ProactorBuilder::new()
        .coop_taskrun(true)
        .capacity(1024)
        .to_owned();
    let rt = compio::runtime::RuntimeBuilder::new()
        .with_proactor(proactor)
        .build()
        .unwrap();
    let start_time = Instant::now();
    let latencies = rt.block_on(async move {
        let mut connection = compio::net::TcpStream::connect("127.0.0.1:2137")
            .await
            .unwrap();
        loop {
            let buf = vec![0; PACKET_SIZE];
            let now = Instant::now();
            let (_, buf) = connection.write_all(buf).await.unwrap();
            let (_, _) = connection.read_exact(buf).await.unwrap();
            let elapsed = now.elapsed();
            results.push(elapsed);
            if results.len() == num_requests {
                break;
            }
        }
        results
    });
    let total_time = start_time.elapsed();
    let throughput_rps = num_requests as f64 / total_time.as_secs_f64();
    
    ClientResults {
        latencies,
        throughput_rps,
    }
}

fn run_monoio_server() {
    let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .build()
        .expect("Failed to build monoio runtime");
    rt.block_on(async {
        let listener = MonoioTcpListener::bind("127.0.0.1:2138").unwrap();
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let shutdown_task = monoio::spawn(async move {
                let mut stream = stream;
                loop {
                    let buf = vec![0u8; PACKET_SIZE];
                    let (result, buf) = stream.read_exact(buf).await;
                    if result.is_err() {
                        break;
                    } else {
                        stream.write_all(buf).await.0.unwrap();
                    }
                }
                return true;
            });
            let shutdown = shutdown_task.await;
            if shutdown {
                break;
            }
        }
    });
}

fn run_monoio_client(num_requests: usize) -> ClientResults {
    let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .build()
        .expect("Failed to build monoio runtime");
    
    rt.block_on(async {
        let mut results = Vec::with_capacity(num_requests);
        let mut stream = MonoioTcpStream::connect("127.0.0.1:2138").await.unwrap();
        let start_time = Instant::now();
        
        for _ in 0..num_requests {
            let buf = vec![0u8; PACKET_SIZE];
            let now = Instant::now();
            let (write_result, buf) = stream.write_all(buf).await;
            write_result.unwrap();
            let (read_result, _) = stream.read_exact(buf).await;
            read_result.unwrap();
            let elapsed = now.elapsed();
            results.push(elapsed);
        }
        
        let total_time = start_time.elapsed();
        let throughput_rps = num_requests as f64 / total_time.as_secs_f64();
        
        ClientResults {
            latencies: results,
            throughput_rps,
        }
    })
}

fn main() {
    println!("=== Compio Benchmark ===");
    let _server_handle = std::thread::spawn(move || {
        // Pin server thread to core 1
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 1 {
            core_affinity::set_for_current(core_ids[1]);
            println!("Server thread pinned to core 1");
        }
        run_compio_server();
    });

    // Give server time to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    let client_results = std::thread::spawn(move || {
        // Pin client thread to core 2
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 2 {
            core_affinity::set_for_current(core_ids[2]);
            println!("Client thread pinned to core 2");
        }
        run_compio_client(1_000_000)
    }).join().unwrap();

    let metrics = PerformanceMetrics::new(client_results);
    metrics.print_report();

    std::thread::sleep(std::time::Duration::from_millis(1000));
    
    println!("\n=== Monoio Benchmark ===");
    let _monoio_server_handle = std::thread::spawn(move || {
        // Pin server thread to core 3 
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 3 {
            core_affinity::set_for_current(core_ids[3]);
            println!("Monoio server thread pinned to core 3");
        }
        run_monoio_server();
    });

    // Give server time to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    let monoio_client_results = std::thread::spawn(move || {
        // Pin client thread to core 4 
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 4 {
            core_affinity::set_for_current(core_ids[4]);
            println!("Monoio client thread pinned to core 4");
        }
        run_monoio_client(1_000_000)
    }).join().unwrap();

    let monoio_metrics = PerformanceMetrics::new(monoio_client_results);
    monoio_metrics.print_report();
}
