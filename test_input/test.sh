#!/bin/sh

ulimit -c unlimited

sudo sysctl -w kernel.core_pattern="|/home/vagrant/test_input/sellafield -u %u -p %p -t %t -E %E -c %c --config /home/vagrant/test_input/sellafield_config.rhai"
test_input/generate_core_dump
echo "Abort finished"

ls -ld test_output
ls -l test_output
