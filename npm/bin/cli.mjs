#!/usr/bin/env node

import { spawn } from "node:child_process";

// Pass all arguments through to `uvx openace`
const args = process.argv.slice(2);

// If no subcommand given, default to "serve ."
if (args.length === 0 || (args.length === 1 && !["index", "search", "serve", "--help", "--version"].includes(args[0]))) {
  args.unshift("serve");
}

const child = spawn("uvx", ["openace", ...args], {
  stdio: "inherit",
  env: { ...process.env },
});

child.on("error", (err) => {
  if (err.code === "ENOENT") {
    process.stderr.write(
      "Error: 'uvx' not found. Install it with: pip install uv\n" +
      "Or install openace directly: pip install openace\n"
    );
    process.exit(1);
  }
  throw err;
});

child.on("exit", (code) => {
  process.exit(code ?? 1);
});
