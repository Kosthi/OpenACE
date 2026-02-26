---
name: deep-search
description: Activates for comprehensive codebase search requests. Triggers when users ask to find all code related to a concept, request thorough/exhaustive search, or need broad coverage across modules. Spawns the deep-researcher agent for multi-round search with gap analysis.
---

When the user requests a comprehensive or thorough codebase search, use the deep-researcher methodology to find all relevant code across the codebase.

## When to Activate

Trigger this skill when the user's request matches any of these patterns:

- **Exhaustive search phrases**: "find all code related to", "find everything that touches", "where is everything that handles", "show me all the code for"
- **Thoroughness indicators**: "thorough search", "comprehensive search", "exhaustive search", "deep search", "complete picture of"
- **Broad coverage requests**: "across the whole codebase", "in all modules", "everywhere that", "all files related to"
- **Understanding requests that imply breadth**: "how does X work end-to-end", "trace the full flow of", "map out everything related to"
- **Multi-module questions**: Questions that clearly span multiple subsystems (e.g., "how does data flow from parsing to storage to retrieval")

## When NOT to Activate

Do **not** activate for:
- Simple lookups ("where is class Foo defined?") -- use `find_symbol` directly
- Single-file questions ("what does engine.py do?") -- use `get_file_outline`
- Narrow, focused searches ("find the authentication middleware") -- use `semantic_search` directly
- Non-search requests (code review, implementation, debugging)

## How to Execute

1. Use the `Task` tool with `subagent_type=Explore` to spawn a subagent
2. Pass the deep-researcher methodology as the prompt (see below)
3. Present the structured output to the user

### Subagent Prompt Template

```
You are performing a deep search of this codebase. Your research question is:

"<USER'S QUESTION>"

Execute a multi-round search with gap analysis. Budget: 15 MCP tool calls max.

**Step 1: Broad Search (2-3 calls)**
- Decompose into 2 semantic_search queries from different angles, limit: 15 each
  - Query A (direct): Use the user's original terms + English translation + domain keywords
  - Query B (functional neighborhood): Rephrase around *what the code does to/with* the concept — upstream inputs, downstream consumers, helper utilities
- Use find_symbol if the user mentions specific names
- Record: unique file paths, directories covered, score distribution

**Step 2: Structural Expansion (2-3 calls)**
- Pick the top 2-3 highest-scoring files from Step 1
- Run get_file_outline on each to discover sibling symbols not caught by semantic search
- Record any new promising symbol names (utilities, helpers, validators, converters) that co-exist in the same files

**Step 3: Gap Analysis (reasoning only, no tool calls)**
Analyze across 7 dimensions:
A. Directory Coverage: Which project areas (model/, backend/, utils/, tests/) have zero hits but should be relevant?
B. Processing Stage Coverage: Are all pipeline stages represented? Think in terms of:
   - Generation: detection models, prediction methods
   - Refinement: post-processing, NMS, filtering, merging, deduplication
   - Transformation: coordinate conversion, format conversion, scaling, rotation correction
   - Consumption: sorting, matching, distance calculation, grouping, association
   - Output: visualization, serialization
C. Indirect Dependency Coverage: Based on file outlines from Step 2, are there utility functions (distance calc, IOU, overlap, interval operations, polygon expansion, perspective transform) that the core code depends on but that didn't appear in semantic results?
D. Edge Case Coverage: Are handlers for special cases represented (rotation/skew correction, occlusion/overlap handling, boundary clipping, empty/invalid filtering)?
E. Score Cliff Detection: If scores drop >50% before position 5, try different terminology
F. Symbol Kind Diversity: If >80% one kind (e.g., all classes, no free functions), target underrepresented kinds
G. Cross-Module Patterns: If a concept (e.g., "bbox", "points") appears in multiple modules with different implementations, ensure all variants are found

**Step 4: Targeted Follow-Up (3-5 calls)**
- For each gap identified in Step 3, craft a targeted query:
  - Indirect utilities: search for function names discovered in file outlines (e.g., "merge_intervals", "calculate_iou", "bbox_distance")
  - Edge cases: search with terms like "angle correction rotation skew" or "occlusion mask overlap removal"
  - Missing stages: search for the missing pipeline stage specifically
- Run semantic_search or find_symbol for each gap, limit: 10
- Merge into working set, dedup by file path
- If >3 new files found with budget remaining, do one more gap analysis round

**Step 5: Depth Pass (2-3 calls)**
- For the most important files (max 3), run get_file_outline to capture the complete symbol inventory
- For each key algorithm function, note: input format, output format, core technique (e.g., "cv2.minAreaRect", "pyclipper offset", "perspective transform")
- This enables the structured output to describe *how* things work, not just *where* they are

**Step 6: Structured Output**
Format the final report with these sections:

### 1. Files by Module
Group files by functional module. For each file list:
- File path and top relevance score
- Key symbols with one-line description of what each does
- For core algorithm functions: input format → technique → output format

### 2. Algorithm Summary Table
For each major algorithm found, one row with:
| Algorithm | File:Function | Input | Core Technique | Output |

### 3. Format & Coordinate Reference
- Box formats used in the codebase (xyxy, xywh, poly-8, points-4x2, etc.) with conversion functions
- Coordinate systems (image, PDF, canvas, normalized) with transformation functions

### 4. Pipeline Flow
A text-based flow diagram showing how boxes move through the system:
Detection → Post-processing → ... → Output

### 5. Coverage Assessment
| Dimension | Status | Notes |
With rows for: directory breadth, pipeline stages, indirect utilities, edge cases, symbol diversity

### 6. Search Metadata
- Queries used with file counts per query
- Tool call count (N/15)
- Confidence: High (>12 files, 4+ modules, all pipeline stages) / Medium (8-12 files, 3+ modules) / Low (<8 files or major gaps)
```

## After Results

- Present the structured output directly to the user
- If confidence is Low or Medium, suggest specific follow-up queries
- If the user needs to investigate specific files further, suggest using `get_file_outline` or reading the files directly
