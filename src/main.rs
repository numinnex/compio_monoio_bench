use std::{
    io::SeekFrom,
    time::{Duration, Instant},
};

use compio::{
    io::{AsyncReadAtExt, AsyncReadExt, AsyncWriteAtExt, AsyncWriteExt},
    net::TcpListener,
    BufResult,
};

use monoio::{
    io::{AsyncReadRentExt, AsyncWriteRentExt},
    net::{TcpListener as MonoioTcpListener, TcpStream as MonoioTcpStream},
};

use tokio::{
    io::{AsyncReadExt as TokioAsyncReadExt, AsyncSeekExt, AsyncWriteExt as TokioAsyncWriteExt},
    net::{TcpListener as TokioTcpListener, TcpStream as TokioTcpStream},
};

const PACKET_SIZE: usize = 512;

// Helper functions to create runtime configurations
fn create_compio_proactor_builder() -> compio::driver::ProactorBuilder {
    compio::driver::ProactorBuilder::new()
        .coop_taskrun(true)
        .taskrun_flag(true)
        .capacity(4096)
        .to_owned()
}

fn create_compio_runtime() -> compio::runtime::Runtime {
    let proactor = create_compio_proactor_builder();
    compio::runtime::RuntimeBuilder::new()
        .with_proactor(proactor)
        .build()
        .unwrap()
}

fn create_monoio_io_uring_runtime() -> monoio::Runtime<monoio::IoUringDriver> {
    monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .build()
        .expect("Failed to build monoio IoUring runtime")
}

fn create_tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[derive(Debug, Clone)]
struct FileMetrics {
    total_operations: usize,
    avg_latency: Duration,
    p50_latency: Duration,
    p90_latency: Duration,
    p99_latency: Duration,
    p999_latency: Duration,
    throughput_ops: f64,
}

impl FileMetrics {
    fn new(latencies: Vec<Duration>, total_time: Duration) -> Self {
        let mut sorted_latencies = latencies.clone();
        let total_operations = sorted_latencies.len();
        sorted_latencies.sort();

        let avg_latency = latencies.iter().sum::<Duration>() / total_operations as u32;
        let p50_latency = sorted_latencies[total_operations * 50 / 100];
        let p90_latency = sorted_latencies[total_operations * 90 / 100];
        let p99_latency = sorted_latencies[total_operations * 99 / 100];
        let p999_latency = sorted_latencies[total_operations * 999 / 1000];
        let throughput_ops = total_operations as f64 / total_time.as_secs_f64();

        Self {
            total_operations,
            avg_latency,
            p50_latency,
            p90_latency,
            p99_latency,
            p999_latency,
            throughput_ops,
        }
    }
}

#[derive(Debug, Clone)]
struct ClientResults {
    // Network metrics
    latencies: Vec<Duration>,
    throughput_rps: f64,

    // File I/O metrics
    sequential_write_metrics: FileMetrics,
    sequential_read_metrics: FileMetrics,
    random_read_metrics: FileMetrics,
    write_then_read_metrics: FileMetrics,
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
            total_time: Duration::from_secs_f64(
                total_requests as f64 / client_results.throughput_rps,
            ),
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
        println!("Network Performance:");
        println!("  total requests: {}", self.total_requests);
        println!("  total time: {:?}", self.total_time);
        println!("  throughput: {:.2} rps", self.throughput_rps);
        println!("  average Latency: {:?}", self.avg_latency);
        println!("  p50 Latency: {:?}", self.p50_latency);
        println!("  p90 Latency: {:?}", self.p90_latency);
        println!("  p99 Latency: {:?}", self.p99_latency);
        println!("  p999 Latency: {:?}", self.p999_latency);
    }

    fn print_file_report(runtime_name: &str, file_metrics: &ClientResults) {
        println!("=== {} File I/O Performance ===", runtime_name);

        println!("Sequential Write:");
        println!(
            "  operations: {}",
            file_metrics.sequential_write_metrics.total_operations
        );
        println!(
            "  throughput: {:.2} ops/s",
            file_metrics.sequential_write_metrics.throughput_ops
        );
        println!(
            "  avg latency: {:?}",
            file_metrics.sequential_write_metrics.avg_latency
        );
        println!(
            "  p50 latency: {:?}",
            file_metrics.sequential_write_metrics.p50_latency
        );
        println!(
            "  p90 latency: {:?}",
            file_metrics.sequential_write_metrics.p90_latency
        );
        println!(
            "  p99 latency: {:?}",
            file_metrics.sequential_write_metrics.p99_latency
        );
        println!(
            "  p999 latency: {:?}",
            file_metrics.sequential_write_metrics.p999_latency
        );

        println!("Sequential Read:");
        println!(
            "  operations: {}",
            file_metrics.sequential_read_metrics.total_operations
        );
        println!(
            "  throughput: {:.2} ops/s",
            file_metrics.sequential_read_metrics.throughput_ops
        );
        println!(
            "  avg latency: {:?}",
            file_metrics.sequential_read_metrics.avg_latency
        );
        println!(
            "  p50 latency: {:?}",
            file_metrics.sequential_read_metrics.p50_latency
        );
        println!(
            "  p90 latency: {:?}",
            file_metrics.sequential_read_metrics.p90_latency
        );
        println!(
            "  p99 latency: {:?}",
            file_metrics.sequential_read_metrics.p99_latency
        );
        println!(
            "  p999 latency: {:?}",
            file_metrics.sequential_read_metrics.p999_latency
        );

        println!("Random Read:");
        println!(
            "  operations: {}",
            file_metrics.random_read_metrics.total_operations
        );
        println!(
            "  throughput: {:.2} ops/s",
            file_metrics.random_read_metrics.throughput_ops
        );
        println!(
            "  avg latency: {:?}",
            file_metrics.random_read_metrics.avg_latency
        );
        println!(
            "  p50 latency: {:?}",
            file_metrics.random_read_metrics.p50_latency
        );
        println!(
            "  p90 latency: {:?}",
            file_metrics.random_read_metrics.p90_latency
        );
        println!(
            "  p99 latency: {:?}",
            file_metrics.random_read_metrics.p99_latency
        );
        println!(
            "  p999 latency: {:?}",
            file_metrics.random_read_metrics.p999_latency
        );

        println!("Write-then-Read:");
        println!(
            "  operations: {}",
            file_metrics.write_then_read_metrics.total_operations
        );
        println!(
            "  throughput: {:.2} ops/s",
            file_metrics.write_then_read_metrics.throughput_ops
        );
        println!(
            "  avg latency: {:?}",
            file_metrics.write_then_read_metrics.avg_latency
        );
        println!(
            "  p50 latency: {:?}",
            file_metrics.write_then_read_metrics.p50_latency
        );
        println!(
            "  p90 latency: {:?}",
            file_metrics.write_then_read_metrics.p90_latency
        );
        println!(
            "  p99 latency: {:?}",
            file_metrics.write_then_read_metrics.p99_latency
        );
        println!(
            "  p999 latency: {:?}",
            file_metrics.write_then_read_metrics.p999_latency
        );
    }
}

fn run_compio_server() {
    let rt = create_compio_runtime();
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

fn run_compio_ping_pong(num_requests: usize) -> (Vec<Duration>, f64) {
    let rt = create_compio_runtime();

    rt.block_on(async move {
        let mut results = Vec::with_capacity(num_requests);
        let mut connection = compio::net::TcpStream::connect("127.0.0.1:2137")
            .await
            .unwrap();
        let start_time = Instant::now();

        for _ in 0..num_requests {
            let buf = vec![0; PACKET_SIZE];
            let now = Instant::now();
            let (_, buf) = connection.write_all(buf).await.unwrap();
            let (_, _) = connection.read_exact(buf).await.unwrap();
            let elapsed = now.elapsed();
            results.push(elapsed);
        }

        let total_time = start_time.elapsed();
        let throughput_rps = num_requests as f64 / total_time.as_secs_f64();

        (results, throughput_rps)
    })
}

fn run_compio_file_benchmark(
    file_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let rt = create_compio_runtime();

    rt.block_on(async move { compio_file_benchmarks(file_operations).await })
}

fn run_compio_client(num_requests: usize, file_operations: usize) -> ClientResults {
    let (latencies, throughput_rps) = run_compio_ping_pong(num_requests);
    let (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    ) = run_compio_file_benchmark(file_operations);

    ClientResults {
        latencies,
        throughput_rps,
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    }
}

fn run_monoio_server() {
    let mut rt = create_monoio_io_uring_runtime();
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

fn run_monoio_ping_pong(num_requests: usize) -> (Vec<Duration>, f64) {
    let mut rt = create_monoio_io_uring_runtime();

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

        (results, throughput_rps)
    })
}

fn run_monoio_file_benchmark(
    file_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let mut rt = create_monoio_io_uring_runtime();

    rt.block_on(async { monoio_file_benchmarks(file_operations).await })
}

fn run_monoio_client(num_requests: usize, file_operations: usize) -> ClientResults {
    let (latencies, throughput_rps) = run_monoio_ping_pong(num_requests);
    let (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    ) = run_monoio_file_benchmark(file_operations);

    ClientResults {
        latencies,
        throughput_rps,
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    }
}

fn run_tokio_server() {
    let rt = create_tokio_runtime();

    rt.block_on(async {
        let listener = TokioTcpListener::bind("127.0.0.1:2139").await.unwrap();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let shutdown_task = tokio::spawn(async move {
                loop {
                    let mut buf = vec![0u8; PACKET_SIZE];
                    let read_result = stream.read_exact(&mut buf).await;
                    if read_result.is_err() {
                        break;
                    } else {
                        stream.write_all(&buf).await.unwrap();
                    }
                }
                return true;
            });
            let shutdown = shutdown_task.await.unwrap();
            if shutdown {
                break;
            }
        }
    });
}

fn run_tokio_ping_pong(num_requests: usize) -> (Vec<Duration>, f64) {
    let rt = create_tokio_runtime();

    rt.block_on(async {
        let mut results = Vec::with_capacity(num_requests);
        let mut stream = TokioTcpStream::connect("127.0.0.1:2139").await.unwrap();
        let start_time = Instant::now();

        for _ in 0..num_requests {
            let buf = vec![0u8; PACKET_SIZE];
            let now = Instant::now();
            stream.write_all(&buf).await.unwrap();
            let mut read_buf = vec![0u8; PACKET_SIZE];
            stream.read_exact(&mut read_buf).await.unwrap();
            let elapsed = now.elapsed();
            results.push(elapsed);
        }

        let total_time = start_time.elapsed();
        let throughput_rps = num_requests as f64 / total_time.as_secs_f64();

        (results, throughput_rps)
    })
}

fn run_tokio_file_benchmark(
    file_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let rt = create_tokio_runtime();

    rt.block_on(async { tokio_file_benchmarks(file_operations).await })
}

fn run_tokio_client(num_requests: usize, file_operations: usize) -> ClientResults {
    let (latencies, throughput_rps) = run_tokio_ping_pong(num_requests);
    let (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    ) = run_tokio_file_benchmark(file_operations);

    ClientResults {
        latencies,
        throughput_rps,
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    }
}

async fn compio_file_benchmarks(
    n_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let filename = "compio_foo";
    let mut data = vec![69u8; PACKET_SIZE];
    let mut pos = 0;
    let mut file = compio::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open(filename)
        .await
        .unwrap();

    // Sequential Write Benchmark
    let mut write_latencies = Vec::with_capacity(n_operations);
    let write_start = Instant::now();

    for _ in 0..n_operations {
        let start = Instant::now();
        let (_, vomited) = file.write_all_at(data, pos).await.unwrap();
        write_latencies.push(start.elapsed());
        pos += vomited.len() as u64;
        data = vomited;
    }
    let write_total_time = write_start.elapsed();
    let sequential_write_metrics = FileMetrics::new(write_latencies, write_total_time);

    // Sequential Read Benchmark
    let mut read_latencies = Vec::with_capacity(n_operations);
    pos = 0;
    let mut read_data = vec![0u8; PACKET_SIZE];
    let read_start = Instant::now();

    for _ in 0..n_operations {
        let start = Instant::now();
        let (_, buf) = file.read_exact_at(read_data, pos).await.unwrap();
        pos += buf.len() as u64;
        read_latencies.push(start.elapsed());
        read_data = buf;
    }
    let read_total_time = read_start.elapsed();
    let sequential_read_metrics = FileMetrics::new(read_latencies, read_total_time);

    // Random Read Benchmark
    let mut random_read_latencies = Vec::with_capacity(n_operations);
    let random_read_start = Instant::now();

    for _ in 0..n_operations {
        let start = Instant::now();
        let max_position = (n_operations - 1) * PACKET_SIZE;
        let position = if max_position > 0 {
            rand::random::<usize>() % max_position
        } else {
            0
        };
        let position = (position / PACKET_SIZE) * PACKET_SIZE; // Align to packet boundaries
        let (_, _) = file
            .read_exact_at(read_data.clone(), position as u64)
            .await
            .unwrap();
        random_read_latencies.push(start.elapsed());
    }
    let random_read_total_time = random_read_start.elapsed();
    let random_read_metrics = FileMetrics::new(random_read_latencies, random_read_total_time);

    // Write-then-Read Benchmark
    let mut write_read_latencies = Vec::with_capacity(n_operations);
    let write_read_start = Instant::now();

    for i in 0..n_operations {
        let start = Instant::now();
        let position = (i * PACKET_SIZE) as u64;
        let (_, buf) = file.write_all_at(data, position).await.unwrap();
        let (_, buf) = file.read_exact_at(buf, position).await.unwrap();
        write_read_latencies.push(start.elapsed());
        data = buf;
    }
    let write_read_total_time = write_read_start.elapsed();
    let write_then_read_metrics = FileMetrics::new(write_read_latencies, write_read_total_time);

    // Cleanup
    std::fs::remove_file(filename).ok();
    std::fs::remove_file("compio_write_read_temp").ok();

    (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    )
}

async fn monoio_file_benchmarks(
    n_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let filename = "monoio_foo";
    let mut data = vec![0u8; PACKET_SIZE];
    let file = monoio::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open(filename)
        .await
        .unwrap();

    // Sequential Write Benchmark
    let mut write_latencies = Vec::with_capacity(n_operations);
    let write_start = Instant::now();

    let mut position = 0;
    for _ in 0..n_operations {
        let start = Instant::now();
        let (result, buf) = file.write_all_at(data, position).await;
        result.unwrap();
        write_latencies.push(start.elapsed());
        position += buf.len() as u64;
        data = buf;
    }
    let write_total_time = write_start.elapsed();
    let sequential_write_metrics = FileMetrics::new(write_latencies, write_total_time);

    // Sequential Read Benchmark
    let mut read_latencies = Vec::with_capacity(n_operations);
    let read_start = Instant::now();

    let mut position = 0;
    for _ in 0..n_operations {
        let start = Instant::now();
        let (result, buf) = file.read_exact_at(data, position).await;
        result.unwrap();
        read_latencies.push(start.elapsed());
        position += buf.len() as u64;
        data = buf;
    }
    let read_total_time = read_start.elapsed();
    let sequential_read_metrics = FileMetrics::new(read_latencies, read_total_time);

    // Random Read Benchmark
    let mut random_read_latencies = Vec::with_capacity(n_operations);
    let random_read_start = Instant::now();

    for _ in 0..n_operations {
        let start = Instant::now();
        let max_position = (n_operations - 1) * PACKET_SIZE;
        let position = rand::random::<usize>() % max_position;
        let (result, buf) = file.read_exact_at(data, position as u64).await;
        result.unwrap();
        random_read_latencies.push(start.elapsed());
        data = buf;
    }
    let random_read_total_time = random_read_start.elapsed();
    let random_read_metrics = FileMetrics::new(random_read_latencies, random_read_total_time);

    // Write-then-Read Benchmark
    let mut write_read_latencies = Vec::with_capacity(n_operations);
    let write_read_start = Instant::now();

    for i in 0..n_operations {
        let start = Instant::now();
        let position = (i * PACKET_SIZE) as u64;
        let (write_result, buf) = file.write_all_at(data, position).await;
        write_result.unwrap();
        let (read_result, buf) = file.read_exact_at(buf, position).await;
        read_result.unwrap();
        write_read_latencies.push(start.elapsed());
        data = buf;
    }
    let write_read_total_time = write_read_start.elapsed();
    let write_then_read_metrics = FileMetrics::new(write_read_latencies, write_read_total_time);

    // Cleanup
    std::fs::remove_file(filename).ok();
    std::fs::remove_file("monoio_write_read_temp").ok();

    (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    )
}

async fn tokio_file_benchmarks(
    n_operations: usize,
) -> (FileMetrics, FileMetrics, FileMetrics, FileMetrics) {
    let filename = "tokio_foo";
    let data = vec![0u8; PACKET_SIZE];
    let mut read_data = vec![0u8; PACKET_SIZE];

    // Sequential Write Benchmark
    let mut write_latencies = Vec::with_capacity(n_operations);
    let write_start = Instant::now();

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .read(true)
        .open(filename)
        .await
        .unwrap();

    let mut pos = 0;
    for _ in 0..n_operations {
        let start = Instant::now();
        file.write_all(&data).await.unwrap();
        file.seek(SeekFrom::Start(pos as u64)).await.unwrap();
        write_latencies.push(start.elapsed());
        pos += data.len();
    }
    file.sync_all().await.unwrap();
    let write_total_time = write_start.elapsed();
    let sequential_write_metrics = FileMetrics::new(write_latencies, write_total_time);

    // Sequential Read Benchmark
    let mut read_latencies = Vec::with_capacity(n_operations);
    let read_start = Instant::now();

    let mut position = 0;
    file.seek(SeekFrom::Start(position)).await.unwrap();
    for _ in 0..n_operations {
        let start = Instant::now();
        file.read_exact(&mut read_data).await.unwrap();
        file.seek(SeekFrom::Start(position)).await.unwrap();
        read_latencies.push(start.elapsed());
        position += read_data.len() as u64;
    }
    let read_total_time = read_start.elapsed();
    let sequential_read_metrics = FileMetrics::new(read_latencies, read_total_time);

    // Random Read Benchmark
    let mut random_read_latencies = Vec::with_capacity(n_operations);
    let random_read_start = Instant::now();

    for _ in 0..n_operations {
        let start = Instant::now();
        let max_position = (n_operations - 1) * PACKET_SIZE;
        let position = rand::random::<usize>() % max_position;
        let position = (position / PACKET_SIZE) * PACKET_SIZE;
        file.seek(SeekFrom::Start(position as u64)).await.unwrap();
        file.read_exact(&mut read_data).await.unwrap();
        random_read_latencies.push(start.elapsed());
    }
    let random_read_total_time = random_read_start.elapsed();
    let random_read_metrics = FileMetrics::new(random_read_latencies, random_read_total_time);

    // Write-then-Read Benchmark
    let mut write_read_latencies = Vec::with_capacity(n_operations);
    let write_read_start = Instant::now();

    let mut position = 0;
    file.seek(SeekFrom::Start(0)).await.unwrap();
    for _ in 0..n_operations {
        let start = Instant::now();
        file.write_all(&data).await.unwrap();
        file.seek(SeekFrom::Start(position)).await.unwrap();

        file.read_exact(&mut read_data).await.unwrap();
        write_read_latencies.push(start.elapsed());
        position += data.len() as u64;
    }

    let write_read_total_time = write_read_start.elapsed();
    let write_then_read_metrics = FileMetrics::new(write_read_latencies, write_read_total_time);

    // Cleanup
    std::fs::remove_file(filename).ok();
    std::fs::remove_file("tokio_write_read_temp").ok();

    (
        sequential_write_metrics,
        sequential_read_metrics,
        random_read_metrics,
        write_then_read_metrics,
    )
}

fn main() {
    let num_network_requests = 1_000_000; 
    let num_file_operations = 100_000; 

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
        run_compio_client(num_network_requests, num_file_operations)
    })
    .join()
    .unwrap();

    let metrics = PerformanceMetrics::new(client_results.clone());
    metrics.print_report();
    PerformanceMetrics::print_file_report("Compio", &client_results);

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
        run_monoio_client(num_network_requests, num_file_operations)
    })
    .join()
    .unwrap();

    let monoio_metrics = PerformanceMetrics::new(monoio_client_results.clone());
    monoio_metrics.print_report();
    PerformanceMetrics::print_file_report("Monoio", &monoio_client_results);

    std::thread::sleep(std::time::Duration::from_millis(1000));

    println!("\n=== Tokio Benchmark ===");
    let _tokio_server_handle = std::thread::spawn(move || {
        // Pin server thread to core 5
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 5 {
            core_affinity::set_for_current(core_ids[5]);
            println!("Tokio server thread pinned to core 5");
        }
        run_tokio_server();
    });

    // Give server time to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    let tokio_client_results = std::thread::spawn(move || {
        // Pin client thread to core 6
        let core_ids = core_affinity::get_core_ids().unwrap();
        if core_ids.len() > 6 {
            core_affinity::set_for_current(core_ids[6]);
            println!("Tokio client thread pinned to core 6");
        }
        run_tokio_client(num_network_requests, num_file_operations)
    })
    .join()
    .unwrap();

    let tokio_metrics = PerformanceMetrics::new(tokio_client_results.clone());
    tokio_metrics.print_report();
    PerformanceMetrics::print_file_report("Tokio", &tokio_client_results);
}
