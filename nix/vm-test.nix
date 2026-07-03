# NixOS VM test for the whereami system module (task-0209).
#
# Boots the real `nixosModules.default` under KVM and machine-verifies the
# security-critical behaviours that tasks 0196 (unix-socket access control) and
# 0197 (state-dir/DB permissions) could only guarantee declaratively:
#   (a) the socket-activated control socket is 0660 root:whereami and the daemon
#       adopts the systemd fd instead of self-binding;
#   (b) a user NOT in allowedUsers is refused (EACCES) on the socket;
#   (c) a user IN allowedUsers gets a valid version response;
#   AC#2  an explicit loopback --bind answers on 127.0.0.1:4747 (+ TCP warning);
#   AC#3/0197  state dir 0700, DB + WAL sidecars 0600;
#   (e) a non-loopback --bind without allowNonlocalBind makes the daemon refuse
#       to start (config eval succeeds; the daemon's validate() rejects it).
#
# Uses `pkgs.testers.runNixOSTest` (the maintained nixpkgs entry point) rather
# than the older `import <nixpkgs/nixos/lib/testing-python.nix>` boilerplate: it
# wires `hostPkgs`/defaults for us and takes the test as a plain module.
{ self, pkgs, gitRev }:

pkgs.testers.runNixOSTest {
  name = "whereami-module";

  nodes = {
    # Main node: the canonical single-owner deployment. allowedUsers grants
    # alice (but not mallory) socket access; bind is the EXPLICIT opt-in TCP
    # listener AC#2 predates but the task re-reads as an opt-in.
    machine = { config, ... }: {
      imports = [ self.nixosModules.default ];
      services.whereami = {
        enable = true;
        allowedUsers = [ "alice" ];
        bind = "127.0.0.1:4747";
      };
      # Put the `whereami` client on PATH so the socket-access assertions use
      # the real client path-resolution, not a hand-rolled socket poke.
      environment.systemPackages = [ config.services.whereami.package ];
      users.users.alice = { isNormalUser = true; };
      users.users.mallory = { isNormalUser = true; };
    };

    # Second node: a non-loopback bind WITHOUT the opt-in flag. There is no
    # nix-side IP parse (config eval succeeds); the daemon's own validate()
    # must reject it at startup and exit non-zero. task-0196 (e).
    # NOTE: node names become Python variables in the test script, so this must
    # NOT be `nonlocal` — that is a reserved Python keyword and the generated
    # `nonlocal.wait_for_unit(...)` would be a syntax error (both at type-check
    # and at runtime).
    nonloopback = { ... }: {
      imports = [ self.nixosModules.default ];
      services.whereami = {
        enable = true;
        bind = "0.0.0.0:4747";
        allowNonlocalBind = false;
      };
    };
  };

  testScript = ''
    start_all()

    # The socket unit is ordered before the service (module: requires=/after=
    # whereamid.socket). Both reaching active, plus the adopting-fd log below,
    # is the ordering evidence.
    with subtest("socket unit and service both reach active"):
        machine.wait_for_unit("whereamid.socket")
        machine.wait_for_unit("whereamid.service")
        # Type=simple reports 'active' the instant the process execs — long
        # before the daemon finishes DB migrations and reaches its listener.
        # Gate every journal assertion below on the daemon's own 'ready' line
        # (logged in run_server AFTER setup_unix_listener adopts the fd and
        # AFTER the TCP bind + warnings), so none of them race daemon startup.
        machine.wait_until_succeeds(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'control channel: unix socket ready'"
        )

    with subtest("(a) control socket is 0660 root:whereami under /run/whereami"):
        machine.succeed("test -d /run/whereami")
        machine.succeed("test -S /run/whereami/whereamid.sock")
        mode = machine.succeed("stat -c %a /run/whereami/whereamid.sock").strip()
        owner = machine.succeed("stat -c %U:%G /run/whereami/whereamid.sock").strip()
        assert mode == "660", f"socket mode is {mode}, expected 660"
        assert owner == "root:whereami", f"socket owner is {owner}, expected root:whereami"

    with subtest("(a) daemon adopted the socket-activated fd and did NOT self-bind"):
        machine.succeed(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'adopting systemd socket-activated fd'"
        )
        # The self-bind path logs 'control channel: unix socket <path>'. On the
        # socket-activation path that line must be absent (the 'unix socket
        # ready' line does not contain 'unix socket /').
        machine.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'control channel: unix socket /'"
        )

    with subtest("(b) non-member (mallory) is refused connecting to the socket"):
        out = machine.fail(
            "runuser -u mallory -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami version --json 2>&1"
        )
        assert "denied" in out.lower() or "permission" in out.lower(), \
            f"expected a permission error, got: {out!r}"

    with subtest("(c) member (alice) gets a valid version response"):
        out = machine.succeed(
            "runuser -u alice -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami version --json"
        )
        assert '"ok":true' in out.replace(" ", ""), \
            f"expected ok:true from alice, got: {out!r}"
        # Pin the WHOLE rev-threading chain end-to-end (flake gitRev ->
        # package preBuild -> build.rs env! -> daemon version response), so a
        # broken embedding can never pass the VM silently (task-0209 review,
        # QA L1). ${gitRev} is the SAME flake binding the module's package
        # was built with.
        assert '"git_rev":"${gitRev}"' in out.replace(" ", ""), \
            f"expected embedded git_rev ${gitRev}, got: {out!r}"

    with subtest("AC#2: explicit TCP bind answers on 127.0.0.1:4747 with a warning"):
        out = machine.succeed("env WHEREAMI_ADDR=127.0.0.1:4747 whereami version --json")
        assert '"ok":true' in out.replace(" ", ""), \
            f"expected ok:true over TCP, got: {out!r}"
        # The opt-in TCP transport must warn loudly that it is unauthenticated.
        machine.succeed(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'TCP listening on 127.0.0.1:4747'"
        )
        machine.succeed(
            "journalctl -u whereamid.service --no-pager | grep -F 'NO peer authentication'"
        )

    with subtest("AC#3/0197: state dir 0700, DB + WAL sidecars 0600"):
        # aps.sqlite appears once the daemon opens the DB; WAL sidecars appear
        # after the first write. Poll for each (they are not there at t=0).
        machine.wait_until_succeeds("test -f /var/lib/whereami/aps.sqlite")
        # /var/lib/whereami is a symlink into /var/lib/private (DynamicUser);
        # stat -L follows it to the real mode.
        dirmode = machine.succeed("stat -L -c %a /var/lib/whereami").strip()
        dbmode = machine.succeed("stat -L -c %a /var/lib/whereami/aps.sqlite").strip()
        assert dirmode == "700", f"state dir mode is {dirmode}, expected 700"
        assert dbmode == "600", f"DB mode is {dbmode}, expected 600"
        for sidecar in ["aps.sqlite-wal", "aps.sqlite-shm"]:
            machine.wait_until_succeeds(f"test -f /var/lib/whereami/{sidecar}")
            m = machine.succeed(f"stat -L -c %a /var/lib/whereami/{sidecar}").strip()
            assert m == "600", f"{sidecar} mode is {m}, expected 600"

    with subtest("(e) non-loopback bind without allowNonlocalBind refuses to start"):
        nonloopback.wait_for_unit("whereamid.socket")
        # The service fails at validate() before adopting the fd; a plain
        # wait_for_unit would hang, so poll is-failed instead.
        nonloopback.wait_until_succeeds("systemctl is-failed --quiet whereamid.service")
        nonloopback.succeed(
            "journalctl -u whereamid.service --no-pager | grep -F -- '--allow-nonlocal-bind'"
        )
  '';
}
