use std::fs;
use std::path::Path;

/// Create a fixture project with 5 languages containing known symbols,
/// relations, and cross-references.
pub fn create_five_language_project(root: &Path) {
    let src = root.join("src");
    let py_dir = src.join("python");
    let ts_dir = src.join("typescript");
    let rs_dir = src.join("rust");
    let go_dir = src.join("go");
    let java_dir = src.join("java");

    for dir in [&py_dir, &ts_dir, &rs_dir, &go_dir, &java_dir] {
        fs::create_dir_all(dir).unwrap();
    }

    // --- Python files ---
    fs::write(
        py_dir.join("models.py"),
        r#"
class User:
    """A user in the system."""

    def __init__(self, name: str, email: str):
        self.name = name
        self.email = email

    def display_name(self) -> str:
        return f"{self.name} <{self.email}>"

class Admin(User):
    """An admin user with elevated permissions."""

    def __init__(self, name: str, email: str, role: str):
        super().__init__(name, email)
        self.role = role

    def has_permission(self, perm: str) -> bool:
        return True

class UserRepository:
    """Manages user persistence."""

    def save(self, user: User) -> bool:
        return True

    def find_by_email(self, email: str) -> User:
        return User("unknown", email)

    def delete(self, user: User) -> bool:
        return True
"#,
    )
    .unwrap();

    fs::write(
        py_dir.join("service.py"),
        r#"
from models import User, UserRepository

class UserService:
    """Business logic for user operations."""

    def __init__(self):
        self.repo = UserRepository()

    def create_user(self, name: str, email: str) -> User:
        user = User(name, email)
        self.repo.save(user)
        return user

    def get_user(self, email: str) -> User:
        return self.repo.find_by_email(email)

    def remove_user(self, email: str) -> bool:
        user = self.get_user(email)
        return self.repo.delete(user)

def process_batch(users: list) -> list:
    svc = UserService()
    return [svc.create_user(u["name"], u["email"]) for u in users]

BATCH_SIZE = 100
"#,
    )
    .unwrap();

    // --- TypeScript files ---
    fs::write(
        ts_dir.join("types.ts"),
        r#"
export interface Config {
    host: string;
    port: number;
    debug: boolean;
}

export interface RequestContext {
    requestId: string;
    timestamp: number;
    config: Config;
}

export type Handler = (ctx: RequestContext) => Promise<void>;

export enum StatusCode {
    OK = 200,
    NotFound = 404,
    ServerError = 500,
}

export const DEFAULT_PORT: number = 8080;
"#,
    )
    .unwrap();

    fs::write(
        ts_dir.join("server.ts"),
        r#"
import { Config, RequestContext, Handler, StatusCode, DEFAULT_PORT } from './types';

class HttpServer {
    private config: Config;
    private handlers: Map<string, Handler>;

    constructor(config: Config) {
        this.config = config;
        this.handlers = new Map();
    }

    register(path: string, handler: Handler): void {
        this.handlers.set(path, handler);
    }

    async handleRequest(ctx: RequestContext): Promise<StatusCode> {
        const handler = this.handlers.get('/');
        if (handler) {
            await handler(ctx);
            return StatusCode.OK;
        }
        return StatusCode.NotFound;
    }

    start(): void {
        console.log(`Server running on ${this.config.host}:${this.config.port}`);
    }
}

function createServer(config?: Partial<Config>): HttpServer {
    const fullConfig: Config = {
        host: config?.host ?? 'localhost',
        port: config?.port ?? DEFAULT_PORT,
        debug: config?.debug ?? false,
    };
    return new HttpServer(fullConfig);
}

export { HttpServer, createServer };
"#,
    )
    .unwrap();

    // --- Rust files ---
    fs::write(
        rs_dir.join("engine.rs"),
        r#"
pub struct SearchEngine {
    name: String,
    max_results: usize,
}

impl SearchEngine {
    pub fn new(name: &str, max_results: usize) -> Self {
        Self {
            name: name.to_string(),
            max_results,
        }
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        vec![SearchResult {
            title: format!("{}: {}", self.name, query),
            score: 1.0,
        }]
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub struct SearchResult {
    pub title: String,
    pub score: f64,
}

pub trait Indexable {
    fn index_key(&self) -> String;
    fn content(&self) -> &str;
}

impl Indexable for SearchResult {
    fn index_key(&self) -> String {
        self.title.clone()
    }
    fn content(&self) -> &str {
        &self.title
    }
}

pub fn create_default_engine() -> SearchEngine {
    SearchEngine::new("default", 100)
}

pub const MAX_QUERY_LENGTH: usize = 1024;
"#,
    )
    .unwrap();

    // --- Go files ---
    fs::write(
        go_dir.join("handler.go"),
        r#"
package handler

type Request struct {
	Method string
	Path   string
	Body   []byte
}

type Response struct {
	Status int
	Body   []byte
}

type Router struct {
	routes map[string]HandlerFunc
}

type HandlerFunc func(req *Request) *Response

func NewRouter() *Router {
	return &Router{routes: make(map[string]HandlerFunc)}
}

func (r *Router) Register(path string, handler HandlerFunc) {
	r.routes[path] = handler
}

func (r *Router) Handle(req *Request) *Response {
	handler, ok := r.routes[req.Path]
	if !ok {
		return &Response{Status: 404, Body: []byte("not found")}
	}
	return handler(req)
}

func DefaultHandler(req *Request) *Response {
	return &Response{Status: 200, Body: []byte("ok")}
}

const MaxBodySize = 1048576
"#,
    )
    .unwrap();

    // --- Java files ---
    fs::write(
        java_dir.join("Application.java"),
        r#"
package com.openace.app;

public class Application {
    private final String name;
    private final int version;

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
        System.out.println("Starting " + this.name + " v" + this.version);
    }

    public static void main(String[] args) {
        Application app = new Application("OpenACE", 1);
        app.start();
    }
}
"#,
    )
    .unwrap();

    fs::write(
        java_dir.join("Service.java"),
        r#"
package com.openace.app;

public interface Service {
    void initialize();
    void execute();
    void shutdown();
}
"#,
    )
    .unwrap();
}

/// Create a large fixture project with `n` files per language for benchmarking.
/// Each file contains a class with a few methods to produce ~10 symbols per file.
pub fn create_scaled_project(root: &Path, files_per_language: usize) {
    let src = root.join("src");

    let py_dir = src.join("python");
    let ts_dir = src.join("typescript");
    let rs_dir = src.join("rust");
    let go_dir = src.join("go");
    let java_dir = src.join("java");

    for dir in [&py_dir, &ts_dir, &rs_dir, &go_dir, &java_dir] {
        fs::create_dir_all(dir).unwrap();
    }

    for i in 0..files_per_language {
        // Python
        fs::write(
            py_dir.join(format!("mod_{i}.py")),
            format!(
                r#"
class Service{i}:
    """Service number {i}."""

    def __init__(self, name: str):
        self.name = name

    def process(self, data: dict) -> dict:
        return {{"name": self.name, "data": data}}

    def validate(self, input_val: str) -> bool:
        return len(input_val) > 0

    def transform(self, value: int) -> int:
        return value * 2

def create_service_{i}() -> Service{i}:
    return Service{i}("svc_{i}")

def helper_{i}(x: int) -> int:
    return x + {i}
"#
            ),
        )
        .unwrap();

        // TypeScript
        fs::write(
            ts_dir.join(format!("mod_{i}.ts")),
            format!(
                r#"
export interface Config{i} {{
    name: string;
    value: number;
}}

export class Handler{i} {{
    private config: Config{i};

    constructor(config: Config{i}) {{
        this.config = config;
    }}

    process(): string {{
        return this.config.name;
    }}

    validate(input: string): boolean {{
        return input.length > 0;
    }}
}}

export function createHandler{i}(name: string): Handler{i} {{
    return new Handler{i}({{ name, value: {i} }});
}}

export const DEFAULT_VALUE_{i}: number = {i};
"#
            ),
        )
        .unwrap();

        // Rust
        fs::write(
            rs_dir.join(format!("mod_{i}.rs")),
            format!(
                r#"
pub struct Component{i} {{
    name: String,
    value: usize,
}}

impl Component{i} {{
    pub fn new(name: &str) -> Self {{
        Self {{ name: name.to_string(), value: {i} }}
    }}

    pub fn process(&self) -> &str {{
        &self.name
    }}

    pub fn transform(&self, x: usize) -> usize {{
        x + self.value
    }}
}}

pub trait Processor{i} {{
    fn execute(&self) -> bool;
}}

impl Processor{i} for Component{i} {{
    fn execute(&self) -> bool {{
        !self.name.is_empty()
    }}
}}

pub fn create_component_{i}() -> Component{i} {{
    Component{i}::new("comp_{i}")
}}
"#
            ),
        )
        .unwrap();

        // Go
        fs::write(
            go_dir.join(format!("mod_{i}.go")),
            format!(
                r#"
package mod{i}

type Service{i} struct {{
	Name  string
	Value int
}}

func NewService{i}(name string) *Service{i} {{
	return &Service{i}{{Name: name, Value: {i}}}
}}

func (s *Service{i}) Process() string {{
	return s.Name
}}

func (s *Service{i}) Transform(x int) int {{
	return x + s.Value
}}

func Helper{i}(x int) int {{
	return x + {i}
}}
"#
            ),
        )
        .unwrap();

        // Java
        fs::write(
            java_dir.join(format!("Mod{i}.java")),
            format!(
                r#"
package com.openace.gen;

public class Mod{i} {{
    private String name;
    private int value;

    public Mod{i}(String name) {{
        this.name = name;
        this.value = {i};
    }}

    public String getName() {{
        return this.name;
    }}

    public int getValue() {{
        return this.value;
    }}

    public int transform(int x) {{
        return x + this.value;
    }}

    public static Mod{i} create() {{
        return new Mod{i}("mod_{i}");
    }}
}}
"#
            ),
        )
        .unwrap();
    }
}
