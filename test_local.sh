#!/bin/sh

# Note this writes to ~/test_output
echo "foo" | cargo run -- -u $(id -u) -p 1 -t 2 -E /foo/bar -c 10000 --config test_input/sellafield_config.rhai
