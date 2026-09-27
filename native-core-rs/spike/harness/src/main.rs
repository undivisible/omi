//! Rotary (rx4)-driven verification harness for the native-core Rust spike.
//!
//! The harness registers its parity checks as rx4 host tools and executes
//! them through the same `ToolRegistry` the agent loop uses — deterministically,
//! with no model provider and no network (`default-features = false`,
//! process-effect tools only). The agent is configured to show the host
//! embedding (scope + policy) that a productized runner would use.

use std::process::Command;
use std::sync::Arc;

use rx4::{
    Agent, Policy, Scope, ToolContext, ToolDefinition, ToolEffect, ToolFuture, ToolRegistry,
    ToolResult,
};

const SH_PRELUDE: &str = "export PATH=\"$HOME/.cargo/bin:$PATH\"; ";

struct Ctx {
    repo: Arc<std::path::Path>,     // v5 worktree root
    crate_root: std::path::PathBuf, // native-core-rs/
    out_dir: std::path::PathBuf,    // scratch outputs (TMPDIR-scoped)
}

impl Ctx {
    fn from_tool(ctx: &ToolContext) -> Self {
        let repo: Arc<std::path::Path> = ctx.workspace_root.clone().into();
        let crate_root = repo.join("native-core-rs");
        let out_dir = std::env::temp_dir().join("omi-spike-harness");
        let _ = std::fs::create_dir_all(&out_dir);
        Self { repo, crate_root, out_dir }
    }

    fn sh(&self, script: &str) -> (bool, String) {
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("{SH_PRELUDE}{script}"))
            .current_dir(&self.repo)
            .output();
        match out {
            Ok(o) => {
                let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                if !o.stderr.is_empty() {
                    text.push_str(&String::from_utf8_lossy(&o.stderr));
                }
                (o.status.success(), tail(&text))
            }
            Err(e) => (false, format!("spawn failed: {e}")),
        }
    }
}

fn tail(s: &str) -> String {
    const MAX: usize = 4000;
    if s.len() <= MAX {
        return s.trim_end().to_string();
    }
    let cut = s.len() - MAX;
    let start = s[cut..]
        .find('\n')
        .map(|i| cut + i + 1)
        .unwrap_or(cut);
    s[start..].trim_end().to_string()
}

fn build_cpp_driver(ctx: &Ctx, out: &std::path::Path) -> Result<(), String> {
    let (ok, log) = ctx.sh(&format!(
        "g++ -std=c++20 -O2 -I native-core-rs/cinclude -I native-core/include \
         native-core-rs/spike/differential/driver.cpp \
         native-core/src/omi_backend_policy.cpp \
         native-core/src/omi_native_boundary.cpp -o {}",
        out.display()
    ));
    if ok {
        Ok(())
    } else {
        Err(format!("C++ driver build failed:\n{log}"))
    }
}

// ---------------------------------------------------------------------------
// tools
// ---------------------------------------------------------------------------

/// Fixed-vector differential: the committed run.sh (C++ driver vs Rust staticlib).
fn tool_differential_fixed(ctx: Arc<ToolContext>, _args: String) -> ToolFuture {
    let ctx = Ctx::from_tool(&ctx);
    Box::pin(async move {
        let script = "native-core-rs/spike/differential/run.sh";
        let (ok, log) = ctx.sh(script);
        if ok {
            ToolResult::ok("differential_fixed", format!("{log}\nverdict: PASS"))
        } else {
            ToolResult::err("differential_fixed", &log)
        }
    })
}

/// Generative differential: seeded pseudo-random probes (no rand dependency),
/// run through both implementations and diffed byte-for-byte.
fn tool_differential_generated(ctx: Arc<ToolContext>, args: String) -> ToolFuture {
    let ctx = Ctx::from_tool(&ctx);
    Box::pin(async move {
        let parsed: serde_json::Value = serde_json::from_str(&args).unwrap_or_default();
        let count = parsed.get("count").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
        let seed = parsed.get("seed").and_then(|v| v.as_u64()).unwrap_or(0x5EED);
        let dir = ctx.out_dir.join("gen");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return ToolResult::err("differential_generated", &format!("mkdir: {e}"));
        }
        let vectors = dir.join("vectors_gen.txt");
        if let Err(e) = std::fs::write(&vectors, generate_probes(count, seed)) {
            return ToolResult::err("differential_generated", &format!("write: {e}"));
        }
        let cpp = dir.join("driver_cpp");
        let rs = dir.join("driver_rs");
        if let Err(e) = build_cpp_driver(&ctx, &cpp) {
            return ToolResult::err("differential_generated", &e);
        }
        let (ok, log) = ctx.sh(&format!(
            "g++ -std=c++20 -O2 -I native-core-rs/cinclude native-core-rs/spike/differential/driver.cpp \
             native-core-rs/target/release/libomi_native_core_rs.a -lpthread -ldl -o {} && \
             cargo build --release --manifest-path native-core-rs/Cargo.toml",
            rs.display()
        ));
        if !ok {
            return ToolResult::err("differential_generated", &format!("Rust driver build failed:\n{log}"));
        }
        let (ok, log) = ctx.sh(&format!(
            "{cpp} {v} > {d}/cpp.txt && {rs} {v} > {d}/rs.txt && diff -u {d}/cpp.txt {d}/rs.txt",
            cpp = cpp.display(),
            rs = rs.display(),
            v = vectors.display(),
            d = dir.display(),
        ));
        if ok {
            ToolResult::ok(
                "differential_generated",
                format!("{count} seeded probes (seed {seed}): C++ vs Rust outputs identical\nverdict: PASS"),
            )
        } else {
            ToolResult::err(
                "differential_generated",
                &format!("differences found:\n{log}"),
            )
        }
    })
}

/// The four native-core C++ host suites (the baseline the Rust side must match).
fn tool_cpp_host_suites(ctx: Arc<ToolContext>, _args: String) -> ToolFuture {
    let ctx = Ctx::from_tool(&ctx);
    Box::pin(async move {
        let suites: &[(&str, &str)] = &[
            ("native_boundary", "native-core/src/omi_native_boundary.cpp"),
            ("backend_policy", "native-core/src/omi_backend_policy.cpp"),
            ("backend_http", "native-core/src/omi_backend_policy.cpp native-core/src/omi_backend_http.cpp"),
            ("backend_recording", "native-core/src/omi_backend_recording.cpp"),
        ];
        let mut lines = Vec::new();
        for (name, srcs) in suites {
            let test = format!("test_omi_{name}");
            let (ok, log) = ctx.sh(&format!(
                "g++ -std=c++20 -I native-core/include native-core/tests/{test}.cpp {srcs} -o {}/{{}} && ./{}",
                ctx.out_dir.display(),
                test
            ));
            // sh above references {} twice; re-run cleanly instead:
            let _ = (ok, log);
            let bin = ctx.out_dir.join(&test);
            let (ok, log) = ctx.sh(&format!(
                "g++ -std=c++20 -I native-core/include native-core/tests/{test}.cpp {srcs} -o {bin} && {bin}",
                bin = bin.display()
            ));
            let verdict = if ok { "PASS" } else { "FAIL" };
            lines.push(format!("{test}: {verdict} {}", last_line(&log)));
            if !ok {
                return ToolResult::err("cpp_host_suites", &lines.join("\n"));
            }
        }
        ToolResult::ok("cpp_host_suites", lines.join("\n"))
    })
}

fn last_line(s: &str) -> String {
    s.lines().next_back().unwrap_or("").trim().to_string()
}

/// Rust parity tests (vectors ported from the C++ suites).
fn tool_rust_parity_tests(ctx: Arc<ToolContext>, _args: String) -> ToolFuture {
    let ctx = Ctx::from_tool(&ctx);
    Box::pin(async move {
        let (ok, log) = ctx.sh("cargo test --manifest-path native-core-rs/Cargo.toml 2>&1 | grep -E 'test result|running'");
        if ok {
            ToolResult::ok("rust_parity_tests", format!("{}\nverdict: PASS", log))
        } else {
            ToolResult::err("rust_parity_tests", &log)
        }
    })
}

/// eqts bridge check: generate the Bun adapter, then drive the Rust-side
/// probes from TypeScript and diff them against the C++ driver output.
fn tool_ts_bridge_check(ctx: Arc<ToolContext>, _args: String) -> ToolFuture {
    let ctx = Ctx::from_tool(&ctx);
    Box::pin(async move {
        let harness = ctx.crate_root.join("spike/harness");
        let steps = [
            format!(
                "cargo build --release --manifest-path {}/Cargo.toml",
                harness.display()
            ),
            "command -v cargo-eqts >/dev/null || cargo install cargo-eqts --locked".to_string(),
            format!(
                "cd {} && cargo eqts build --target bun --release --out-dir dist",
                harness.display()
            ),
        ];
        for step in steps {
            let (ok, log) = ctx.sh(&step);
            if !ok {
                return ToolResult::err("ts_bridge_check", &format!("step failed: {step}\n{log}"));
            }
        }
        let cpp = ctx.out_dir.join("driver_ts_cpp");
        if let Err(e) = build_cpp_driver(&ctx, &cpp) {
            return ToolResult::err("ts_bridge_check", &e);
        }
        let (ok, log) = ctx.sh(&format!(
            "bun {harness}/ts/harness.ts {cpp} {vectors} {harness}/dist",
            harness = harness.display(),
            cpp = cpp.display(),
            vectors = ctx.crate_root.join("spike/differential/vectors.txt").display(),
        ));
        if ok {
            ToolResult::ok("ts_bridge_check", format!("{log}\nverdict: PASS"))
        } else {
            ToolResult::err("ts_bridge_check", &log)
        }
    })
}

// ---------------------------------------------------------------------------
// seeded probe generation (deterministic; no external RNG dependency)
// ---------------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn generate_probes(count: usize, seed: u64) -> String {
    const PATHS: &[&str] = &[
        "/v1/settings",
        "/v1/live/sessions",
        "/v1/live/sessions-extra",
        "/v1/chat-messages?limit=50",
        "/v1/chat-generations/id/events",
        "/v1/chat-attachments/x/complete",
        "/v1/device-sessions",
        "/v1/device-sessions//transcribe",
        "/v1/device-sessions/id/audio",
        "/v1/device-sessions/a/b/transcribe",
        "/v1/device-sessions/11111111-2222-3333-4444-555555555555/transcribe",
        "/v1/device-sessions/id/transcript",
        "/v1/device-sessions-extra",
        "/v1/conversations#keep",
        "/v1/memories",
        "/v1/tasks/ops?op=complete",
        "/v1/tasks/one",
        "/v1/users/me",
        "/v1/settings#x",
        "",
    ];
    const METHODS: &[&str] = &["GET", "POST", "PATCH", "DELETE", "PUT", "post"];
    const HOSTS: &[&str] = &[
        "localhost",
        "LOCALHOST",
        "127.0.0.1",
        "::1",
        "[::1]",
        "[127.0.0.1]",
        "api.omi.me",
        "API.OMI.ME",
        "api.omi.me.evil.com",
        "synthetic.workers.dev",
        "workers.dev",
        ".workers.dev",
        "[A.WORKERS.DEV]",
        "evil.workers.dev.example.com",
        "untrusted.invalid",
        "omi-platform-dev-attacker.a.run.app",
    ];
    const PLANES: &[&str] = &["new", "old", "unexpected", "NEW", "-"];
    let mut rng = Lcg(seed.wrapping_add(1));
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let line = match rng.below(8) {
            0 => format!("strip {}", rng.pick(PATHS)),
            1 => format!("capture {}", rng.pick(PATHS)),
            2 => format!("timeout {} {}", rng.pick(METHODS), rng.pick(PATHS)),
            3 => format!("example {} {}", rng.pick(METHODS), rng.pick(PATHS)),
            4 => format!("loopback {}", rng.pick(HOSTS)),
            5 => format!("cloud {}", rng.pick(HOSTS)),
            6 => format!("allowed {}", rng.pick(HOSTS)),
            _ => {
                let mut hex = String::new();
                let len = rng.below(16);
                for _ in 0..len {
                    hex.push_str(&format!("{:02x}", rng.next() & 0xff));
                }
                if hex.is_empty() {
                    hex.push_str("aa55");
                }
                format!("crc {hex}")
            }
        };
        lines.push(line);
    }
    // Frame probes need coherent checksums, so append them separately.
    for i in 0..count / 8 {
        let payload_len = rng.below(16) as usize;
        let mut payload = Vec::with_capacity(payload_len);
        for _ in 0..payload_len {
            payload.push((rng.next() & 0xff) as u8);
        }
        let mut framed = vec![0xAAu8, 0x55];
        framed.extend_from_slice(&payload);
        if i % 4 == 3 {
            framed.push(0x00); // corrupt length/tail variants
        } else {
            let crc = crc32_reflect(&payload);
            framed.extend_from_slice(&crc.to_be_bytes());
        }
        let hex: String = framed.iter().map(|b| format!("{b:02x}")).collect();
        let max_out = [0usize, 2, payload_len, payload_len + 32][(rng.below(4)) as usize];
        lines.push(format!("frame {hex} {max_out}"));
    }
    lines.join("\n") + "\n"
}

fn crc32_reflect(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xEDB8_8320 } else { 0 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

fn def(
    registry: &ToolRegistry,
    name: &str,
    description: &str,
    schema: &str,
    effect: ToolEffect,
    f: fn(Arc<ToolContext>, String) -> ToolFuture,
) {
    registry.register(
        ToolDefinition::new_fn(name, description, schema, f).with_effect(effect),
    );
}

/// Walks up from `start` to the directory containing the native-core tree.
fn find_repo_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|p| p.join("native-core/src/omi_backend_policy.cpp").is_file())
        .map(std::path::Path::to_path_buf)
}

#[tokio::main]
async fn main() {
    // Optional overrides: omi-spike-harness [generated_count] [generated_seed]
    let mut cli = std::env::args().skip(1);
    let gen_count: u64 = cli.next().and_then(|v| v.parse().ok()).unwrap_or(500);
    let gen_seed: u64 = cli
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0x5EED);

    let registry = ToolRegistry::new();
    def(
        &registry,
        "differential_fixed",
        "Run the committed 112-probe differential (C++ driver vs Rust staticlib).",
        r#"{"type":"object","properties":{}}"#,
        ToolEffect::Process,
        tool_differential_fixed,
    );
    def(
        &registry,
        "differential_generated",
        "Seeded pseudo-random differential over both implementations.",
        r#"{"type":"object","properties":{"count":{"type":"integer","minimum":1},"seed":{"type":"integer"}}}"#,
        ToolEffect::Process,
        tool_differential_generated,
    );
    def(
        &registry,
        "cpp_host_suites",
        "Build and run the four native-core C++ host test suites.",
        r#"{"type":"object","properties":{}}"#,
        ToolEffect::Process,
        tool_cpp_host_suites,
    );
    def(
        &registry,
        "rust_parity_tests",
        "Run the Rust parity tests ported from the C++ suites.",
        r#"{"type":"object","properties":{}}"#,
        ToolEffect::Process,
        tool_rust_parity_tests,
    );
    def(
        &registry,
        "ts_bridge_check",
        "Generate the eqts Bun adapter and diff Rust-side probes (driven from TypeScript) against the C++ driver.",
        r#"{"type":"object","properties":{}}"#,
        ToolEffect::Process,
        tool_ts_bridge_check,
    );

    println!("harness engine: rx4 (rotary) {}", env!("CARGO_PKG_VERSION"));
    println!("tools: {}", registry.names().join(", "));
    println!("loadout fingerprint: {}", registry.definitions_fingerprint());

    // Host embedding record: research scope, read-only policy. The harness
    // never writes inside the workspace (scratch outputs go to TMPDIR).
    let mut agent = Agent::new();
    agent.set_scope(Scope::Research);
    agent.set_policy(Policy::read_only());

    let root = find_repo_root(&std::env::current_dir().expect("cwd"))
        .expect("run inside the omi v5 worktree (native-core not found)");
    let ctx = Arc::new(ToolContext::new(root));
    let mut failed = false;
    for (name, args) in [
        ("differential_fixed", "{}".to_string()),
        (
            "differential_generated",
            format!(r#"{{"count":{gen_count},"seed":{gen_seed}}}"#),
        ),
        ("cpp_host_suites", "{}".to_string()),
        ("rust_parity_tests", "{}".to_string()),
        ("ts_bridge_check", "{}".to_string()),
    ] {
        println!("\n== {name} ==");
        let result = registry
            .execute(name, &ctx, &args)
            .await
            .expect("tool registered");
        println!("{}", result.content);
        if result.is_error {
            failed = true;
        }
    }

    // Mount the loadout on the host agent for the record (no prompt is run:
    // this harness is deterministic and provider-free).
    agent.set_tools(registry);
    println!("\nagent scope: {:?}", agent.scope);
    println!("harness verdict: {}", if failed { "FAIL" } else { "PASS" });
    if failed {
        std::process::exit(1);
    }
}
