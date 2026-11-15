---
name: tdd
description: Test-driven development.
agents: []
pre-loaded: [backend_engineer, frontend_engineer, automation_engineer]
---

# Test-Driven Development

TDD is the red → green loop. This skill is the reference that makes that loop produce tests worth keeping: what a good test is, where tests go, the anti-patterns, and the rules of the loop. You are required to follow TDD on every task — write a test before implementing the code.

Test names and interface vocabulary should match the project's domain language and respect the invariants defined in the module's LLD.

## What a good test is

Tests verify behavior through public interfaces, not implementation details. Code can change entirely; tests shouldn't. A good test reads like a specification — "user can checkout with valid cart" tells you exactly what capability exists — and survives refactors because it doesn't care about internal structure.

See `.echelon/skills/tdd/tests.md` (read via `read('.echelon/skills/tdd/tests.md')`) for examples and `.echelon/skills/tdd/mocking.md` for mocking guidelines.

## Seams — where tests go

A **seam** is the public boundary you test at: the interface where you observe behavior without reaching inside. Tests live at seams, never against internals.

Think: "What's the public interface, and what seam should I test?"

To understand the shape of that interface — how deep the module is, where the seam belongs, what the interface should expose — refer to the LLD of the module.

## Anti-patterns

- **Implementation-coupled** — mocks internal collaborators, tests private methods, or verifies through a side channel.
- **Tautological** — the assertion recomputes the expected value the way the code does (`expect(add(a, b)).toBe(a + b)`), so it passes by construction and can never disagree with the code. Expected values must come from an independent source of truth.

## Rules of the loop

- **Red before green.** Write the failing test first, then run it via `bash()` to confirm it actually fails — a test that passes before any implementation is testing the wrong thing. Then write only enough code to make it pass.
- **One slice at a time.** One seam, one test, one minimal implementation per cycle.
