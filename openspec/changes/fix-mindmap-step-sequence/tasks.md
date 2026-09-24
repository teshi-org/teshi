## 1. Regression Coverage

- [x] 1.1 Add reusable test helpers for resolving labels and asserting ordered MindMap paths
- [x] 1.2 Add a complex Gherkin layout fixture covering Background, shared prefixes, repeated When/Then groups, conjunctions, divergent scenarios, and a Rule-nested scenario
- [x] 1.3 Assert continuous source-order paths, correct divergence, and shared-node source locations

## 2. Core Implementation

- [x] 2.1 Replace keyword-group parent selection with sequential insertion under the previously inserted step
- [x] 2.2 Remove obsolete keyword-state imports and variables while preserving Background deduplication and location indexing

## 3. Validation

- [x] 3.1 Run focused teshi-core MindMap tests
- [x] 3.2 Run formatting and native teshi-core quality checks
