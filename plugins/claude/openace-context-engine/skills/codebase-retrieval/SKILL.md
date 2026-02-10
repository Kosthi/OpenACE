---
name: codebase-retrieval
description: This skill should be used when the user asks about their codebase, needs to find code locations, or wants to understand how their code works. Activates for questions about code structure, finding functions/classes, understanding implementations, or locating where to make changes.
---

When the user asks about their codebase, use the OpenACE tools (`semantic_search`, `find_symbol`, `get_file_outline`) to find relevant code instead of guessing or asking the user to provide file paths.

## When to Use This Skill

Activate this skill when the user:

- Asks where something is implemented ("Where is user authentication handled?")
- Needs to find code locations ("Find the database connection code")
- Wants to understand how something works ("How does the payment processing work?")
- Needs to make changes and doesn't know where ("I need to add a new API endpoint")
- Asks about code structure ("What tests exist for the login feature?")
- Mentions specific concepts that need to be located in the codebase

## How to Use Codebase Retrieval

### Step 1: Formulate Effective Queries

Create search queries based on the user's request:

- **Direct queries**: Use function names, class names, or variable names if mentioned
- **Conceptual queries**: Describe what the code does ("user authentication", "database connection")
- **Related patterns**: Search for similar functionality or related concepts

### Step 2: Choose the Right Tool

Select the tool that best matches the user's request:

- **`find_symbol`**: Use when the user mentions a specific name -- a class, function, variable, or type. This performs an exact name lookup and returns the symbol's file path, line range, and signature.
- **`semantic_search`**: Use when the user describes a concept or behavior rather than a specific name. This searches by natural language and returns ranked results with relevance scores, file paths, line ranges, and related symbols.
- **`get_file_outline`**: Use when the user wants to understand what a particular file contains or needs an overview of a file's structure. This returns all symbols in the file with their kinds, names, line ranges, and signatures.

### Step 3: Search the Codebase

Call the chosen tool with the appropriate input:

**`semantic_search`** -- pass a natural language description of the code you're looking for:
- "Where is the function that handles user authentication?"
- "What tests are there for the login functionality?"
- "How is the database connected to the application?"
- "Where are API endpoints defined for user management?"

**`find_symbol`** -- pass the exact name of the symbol:
- "AuthService"
- "handleLogin"
- "DatabasePool"
- "UserController"

**`get_file_outline`** -- pass the relative file path:
- "src/auth/service.ts"
- "lib/database/pool.rs"

### Step 4: Analyze and Iterate

From the retrieval results:

- Review the returned code snippets for relevance
- If results aren't helpful, try alternative queries with different terminology
- Search for related concepts if the direct search doesn't yield results
- Combine tools when needed: use `semantic_search` to locate a file, then `get_file_outline` to understand its structure, then `find_symbol` for specific definitions

### Step 5: Present Findings

Provide clear, actionable information:

- List relevant files with their paths
- Explain what each file/function does
- Point to specific functions, classes, and line ranges when possible
- Suggest which code should be modified for the requested change

## Guidelines

- **Be thorough**: Search using multiple related queries if the first doesn't yield complete results
- **Be specific**: Pass detailed queries for better retrieval results
- **Stay current**: The retrieval tools reflect the current state of the codebase on disk
- **Avoid guessing**: Always search before making assumptions about code locations
- **Iterate**: If initial results aren't helpful, reformulate your query and try again
