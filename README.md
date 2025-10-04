# bestbind

(Old name: admirror-speedtest, rsync-speedtest)

A simple speedtest program for multiple-IPs (ISP) environment, to optimize the speed of syncing from upstream. **Supports rsync, curl, wget and git; you can also test with Docker/Podman networks**.

PS: There's a racing bug in rsync that prevents proper termination of rsync processes, and it has been workarounded in rsync-speedtest. See comments of [`kill_children()` in main.rs](src/main.rs) for details.

## Args

```console
$ ./bestbind --help
Test speed (bandwidth) of different bind IP to rsync, http(s) and git upstream. Alleviate mirror site admin's trouble choosing fastest bind IP.

Usage: bestbind [OPTIONS] <UPSTREAM>

Arguments:
  <UPSTREAM>  Upstream path. Will be given to specified program

Options:
      --profile <PROFILE>  Profile name in config file. If not given, it will use "default" profile [default: default]
  -c, --config <CONFIG>    Config file (IP list) path. Select order is bestbind.conf in XDG config, then ~/.bestbind.conf, then /etc/bestbind.conf
  -p, --pass <PASS>        Passes number [default: 3]
  -t, --timeout <TIMEOUT>  Timeout (seconds) [default: 30]
      --tmp-dir <TMP_DIR>  Tmp file path. Default to `env::temp_dir()` (/tmp in Linux system)
      --log <LOG>          Log file. Default to /dev/null When speedtesting, the executed program output is redirected to this file [default: /dev/null]
      --program <PROGRAM>  Program to use. It will try to detect by default (here curl will be used default for http(s)) [possible values: rsync, wget, curl, git]
      --extra <EXTRA>      Extra arguments. Will be given to specified program
  -h, --help               Print help
  -V, --version            Print version
```

### Git support

`libbinder.so` will be searched by this order:

- `/usr/lib/bestbind/libbinder.so`
- The path `LIBBINDER_PATH` env var points to

Note that libbinder is now seperated to another repo: <https://github.com/taoky/libbinder>, with glibc & musl support.

It throws error and git support will not be available if `libbinder.so` is not found.

## Config file format

Format from 0.4.0 is not compatible with previous versions.

See [assets/bestbind.conf.example](assets/bestbind.conf.example) for example.

## Screenshot

![Screenshot](assets/demo.png)
