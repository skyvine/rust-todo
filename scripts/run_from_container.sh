#!/bin/sh
set -e

~/.cargo/bin/diesel migration run
exec /app/rust-todo "$@"
