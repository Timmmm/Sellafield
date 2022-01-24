# Sellafield - A customisable core dumper

Sellafield is a tool you can use to customise how core dumps are saved. You just set your `core_pattern` like this:

    sysctl -w kernel.core_pattern="|/path/to/sellafield -u %u -p %p -t %t -E %E -c %c --config /path/to/config"

Then create `/path/to/config` which is a [Rhai](https://rhai.rs/) script that writes out the core file. The Rhai script has access to these functions:

* `home()` - Home directory
* `username()` - Username
* `uid()` - UID
* `pid()` - PID
* `time()` - Time in Epoch millis
* `full_exe()` - Full path to the crashed executable
* `exe()` - Name of the crashed executable

And it can call these functions to affect how the core is dumped.

* `set_output_path(string)` - Set the path to save to.
* `set_permissions(int)` - Set the permissions to use for the file.

For example you might have a very simple script like this:

```rhai
set_output_path(`${home()}/.core_dumps/core.${pid()}.${exe()}`);
set_permissions(0o404);
```

If you don't want to write the core file, don't call `set_output_path()` (or call `set_output_path("")`.

Note that the returned `path` *can* be relative, but it will always be relative to the root directory so there's probably no point.

## Per-User Config

You can do per-user config by changing the main config file to import a user-config file (and catch errors if it doesn't exist).

```rhai
// Set default first.
set_output_path(`${home()}/.core_dumps/${pid()}.${exe()}`);

try {
    import `${home()}/.config/sellafield`;
} catch {
    // Ignore errors.
}
```

## Errors

Unfortunately when commands are run as part of a core pattern their stdout and stderr are sent to `/dev/null`, so debugging them can be very tricky! To make this a bit easier, if there are any errors then they are logged to `/tmp/sellafield_<epoch time>.log`.

## Build

I recommend using the `x86_64-unknown-linux-musl` target because then it doesn't depend on Glibc and inherit all its issues. On Mac you will need a cross-compiler:

    brew install filosottile/musl-cross/musl-cross

And you need the musl target:

    rustup target add x86_64-unknown-linux-musl

Then you can just run

    cargo build --release --target x86_64-unknown-linux-musl

## Install

Either download a binary release, or copy the output binary in `target/x86_64-unknown-linux-musl/release/sellafield` somewhere (ideally somewhere with a short path since the core pattern is limited to 127 characters), and then set the core pattern as described above.

## Test

Testing cannot be done using Docker (at least not on Linux) because it doesn't use its own kernel. Instead we can use QEMU via [Transient](https://github.com/ALSchwalm/transient).

    # Install requirements.
    pip3 install transient

    # Build.
    cargo build --release --target x86_64-unknown-linux-musl

    # Copy to test directory.
    cp target/x86_64-unknown-linux-musl/release/sellafield test_input/sellafield

    transient run centos/7:2004.01 \
        --copy-in-before test_input:/home/vagrant/test_input \
        --ssh-command /home/vagrant/test_input/test.sh \
        -- -m 1G

This is all wrapped up in the `vm_test` crate so you can just do:

    cd vm_test && cargo run
