use criterion::{criterion_group, criterion_main, Criterion, Throughput};

/// Benchmark: parser throughput (target >50K symbols/sec)
fn bench_parser_throughput(c: &mut Criterion) {
    // A medium-sized Python file producing ~10 symbols
    let python_source = r#"
class UserService:
    """Service for user management."""

    def __init__(self, repo):
        self.repo = repo

    def create_user(self, name: str, email: str) -> dict:
        return {"name": name, "email": email}

    def delete_user(self, user_id: int) -> bool:
        return True

    def find_user(self, email: str) -> dict:
        return self.repo.find(email)

    def update_user(self, user_id: int, data: dict) -> dict:
        return data

    def list_users(self) -> list:
        return []

class AdminService(UserService):
    def promote(self, user_id: int) -> bool:
        return True

    def demote(self, user_id: int) -> bool:
        return True

def process_request(data: dict) -> dict:
    svc = UserService(None)
    return svc.create_user(data["name"], data["email"])

BATCH_SIZE = 100
MAX_USERS = 10000
"#;

    let ts_source = r#"
export interface Config {
    host: string;
    port: number;
}

export class Server {
    private config: Config;

    constructor(config: Config) {
        this.config = config;
    }

    start(): void {
        console.log('starting');
    }

    stop(): void {
        console.log('stopping');
    }

    restart(): void {
        this.stop();
        this.start();
    }
}

export function createServer(host: string, port: number): Server {
    return new Server({ host, port });
}

export const DEFAULT_HOST = 'localhost';
export const DEFAULT_PORT = 8080;
"#;

    let rust_source = r#"
pub struct Engine {
    name: String,
    running: bool,
}

impl Engine {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), running: false }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub trait Runnable {
    fn run(&self) -> bool;
}

impl Runnable for Engine {
    fn run(&self) -> bool {
        self.running
    }
}

pub fn create_engine(name: &str) -> Engine {
    Engine::new(name)
}
"#;

    let go_source = r#"
package main

type Handler struct {
    Name string
    Active bool
}

func NewHandler(name string) *Handler {
    return &Handler{Name: name, Active: true}
}

func (h *Handler) Handle() string {
    return h.Name
}

func (h *Handler) IsActive() bool {
    return h.Active
}

func (h *Handler) Deactivate() {
    h.Active = false
}

func ProcessRequest(name string) string {
    h := NewHandler(name)
    return h.Handle()
}
"#;

    let java_source = r#"
package com.example;

public class Application {
    private String name;
    private int version;

    public Application(String name, int version) {
        this.name = name;
        this.version = version;
    }

    public String getName() {
        return this.name;
    }

    public int getVersion() {
        return this.version;
    }

    public void start() {
        System.out.println("Starting");
    }

    public void stop() {
        System.out.println("Stopping");
    }

    public static void main(String[] args) {
        Application app = new Application("test", 1);
        app.start();
    }
}
"#;

    let sources: Vec<(&str, &str, &[u8])> = vec![
        ("test-repo", "src/service.py", python_source.as_bytes()),
        ("test-repo", "src/server.ts", ts_source.as_bytes()),
        ("test-repo", "src/engine.rs", rust_source.as_bytes()),
        ("test-repo", "src/handler.go", go_source.as_bytes()),
        ("test-repo", "src/App.java", java_source.as_bytes()),
    ];

    // Count total symbols from a single pass to set throughput
    let total_symbols: usize = sources
        .iter()
        .map(|(repo, path, content)| {
            oc_parser::parse_file(repo, path, content, content.len() as u64)
                .map(|o| o.symbols.len())
                .unwrap_or(0)
        })
        .sum();

    let mut group = c.benchmark_group("parser_throughput");
    group.throughput(Throughput::Elements(total_symbols as u64));
    group.bench_function("parse_5_languages", |b| {
        b.iter(|| {
            for (repo, path, content) in &sources {
                let _ = oc_parser::parse_file(repo, path, content, content.len() as u64);
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parser_throughput);
criterion_main!(benches);
