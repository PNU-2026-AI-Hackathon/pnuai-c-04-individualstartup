# CADGen-AX Development Guidelines

This document defines the repository-level development standards shared by team members and AI coding agents. The guidelines were derived from practices repeatedly applied in the project's issues, pull requests, and code reviews.

## 1. Strict Fail-Fast Policy

- Do not add fallback behavior that hides errors or silently changes execution paths.
- Do not return placeholders, empty results, or artificial success responses after a failure.
- Do not add mock, fake, or stub data to production code to make incomplete features appear functional.
- Use mocks and stubs only in tests. Restore any modified global state or prototypes when each test finishes.
- Preserve the original cause of invalid input, external process failures, and initialization errors, and expose it explicitly to the caller or UI.
- Record a quality-check failure separately from a failure to execute the check itself.

## 2. Scope and Issue Definition

- Before implementation, document the background, reproduction steps or objective, requirements, acceptance criteria, and out-of-scope items in an issue.
- Split large changes into subtasks that can be implemented and verified independently.
- Modify only what is required by the current acceptance criteria. Do not mix unrelated refactoring or behavior changes into the same change.
- When existing behavior changes, state which contracts and user flows are affected.

## 3. Architecture and State Management

- Keep the responsibilities of the React/TypeScript UI, Tauri/Rust services, modeling plane, and validation plane separate.
- Explicitly preserve the relationships among sessions, revisions, sources, runs, validation batches, and artifacts.
- Do not mutate finalized validation inputs or artifacts during execution. Keep them traceable through hashes and revision history.
- Represent asynchronous work with explicit state transitions such as `queued`, `running`, `succeeded`, and `failed`.
- Enforce guarantees through code and data contracts rather than relying only on prompts or UI conventions.

## 4. Implementation Standards

- When catching an exception, perform the necessary cleanup and then propagate the error without losing its original meaning or cause.
- If asynchronous initialization fails midway, do not leave behind DOM nodes, processes, listeners, observers, or scheduled work.
- Define ownership of WebGL geometry, materials, and textures, and dispose of each resource exactly once on replacement, failure, or unmount.
- Prefer state-driven execution over continuous polling or render loops. Prevent duplicate scheduling and callbacks after cleanup.
- For UI changes, consider keyboard controls, focus behavior, accessible names, and narrow viewports in addition to pointer interaction.
- File paths, build scripts, and CI workflows must behave consistently across supported macOS, Windows, and Linux environments.
- Document the rationale for domain-specific constants such as tolerances, angles, and resolution limits.

## 5. Verification and Testing

- Add a reproduction test for every bug fix. Cover success, failure, and boundary conditions for new features.
- Test user-visible contracts, state transitions, input immutability, and resource lifecycles rather than only implementation details.
- Verify that errors reach the caller or Error Boundary and that the system remains in a valid state for retry after a failure.
- Scope test mocks to individual tests. Do not let them suppress errors or alter global behavior for other tests.
- Use justified tolerances for floating-point and geometry calculations. Avoid exact comparisons that are fragile across platforms.
- Run the commands appropriate to the change scope:

```bash
npm run check
npm test
npm run build
npm run test:rust   # When Rust code or backend contracts change
```

- When automated tests are insufficient for rendering, CAD geometry, or cross-platform packaging, verify representative samples in the actual target environment. Record the environment, results, and known limitations in the pull request.

## 6. Pull Requests and Reviews

- Include the related issue, change summary, key design decisions, verification performed, and known limitations in each pull request.
- Separate blocking changes, recommended improvements, and clarification questions in reviews.
- Compare review suggestions against the actual code and requirements. Document the resulting change and verification for accepted suggestions, and provide a technical reason for suggestions that are not applied.
- After addressing review feedback, rerun both the relevant tests and the full test or build suite appropriate to the impact area.
- Before merging, a team member must inspect the diff, architecture boundaries, failure paths, and test results. Code written or reviewed by AI is not exempt from this requirement.

## 7. Codex Usage

- Codex may assist with requirement analysis, implementation, refactoring, testing, documentation, issue and pull-request writing, and code review.
- In the product, Codex may perform natural-language CAD modeling and validation-driven refinement, but state transitions and success decisions must follow explicit application contracts.
- Developers should reduce time spent on repetitive code navigation and prioritize review time for architecture, interfaces, failure conditions, and test design.
- Treat Codex output as a draft or proposed change. Team members remain responsible for accepting completion criteria and approving merges.
