---
name: implement-ticket
description: Make the changes requried to meet a ticket's acceptance criteria
metadata:
    opencode/slash: "true"
    opencode/autoinvoke: "false"
---

# Implement a ticket

Ticket: $ARGUMENTS

Work through these steps in order. Don't make any code changes until you reach the appropriate step. If at any step you find an unrelated problem, communicate this to the user and ask how to handle. Continue working in the background while you wait for the user's response.

## 1. Understand
- Read the ticket fully.
- Explore the code it touches: impacted files, existing callers, and impacted tests.
- Restate the problem and acceptance criteria in two or three sentences, and ask the user to sign off on your summary.

## 2. Clarify (if needed)
- Ask questions to resolve ambiguity if it would impact the implementation.
- Consider edge cases and security implications. Ask the user how to handle them when unsure, or when the decision is a policy rather than technical decision.
- Append any findings to the ticket.
- If it's a bug, reproduce the behavior.

## 3. Implement
- Make the smallest change that resolves the ticket, following repository conventions.
- If you are duplicating code already found, consider refactoring to a helper function.
- Add tests covering the fixes and the edge cases.

## 4. Verify
- Make sure all of these pass:
  - `cargo test`
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
- If it's a bug, try to reproduce the behavior again.
- If something fails and it's outside the ticket's scope, ask the user for guidance.

## 5. Report
Supply the user with a summary containing:
- What changed and why
- How it was tested and the results

## Additional Context
- Leave changes uncommitted in the working directory. Do not commit changes unless explicitly asked to do so.
- Check off the ticket's acceptance criteria as you go. Don't finish until all of the acceptance criteria is checked. If you are unable to resolve one of them, ask the user for guidance.
