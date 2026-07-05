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

let
  # A byte-for-byte copy at a distinct store path. The scan-node mount namespace
  # overlays the packaged nmcli destination, so invoking that path recursively
  # would only call the shim again. This copy genuinely connects to the system
  # bus and NetworkManager before any deterministic fixture is emitted.
  realNmcliProbe = pkgs.runCommand "whereami-real-nmcli-probe" { } ''
    install -Dm755 ${pkgs.networkmanager}/bin/nmcli $out/bin/nmcli-real
  '';

  # Hardware-independent kernel-path proof for the iw fallback. Opening a
  # NETLINK_GENERIC socket exercises both RestrictAddressFamilies=AF_NETLINK and
  # the active SystemCallFilter without needing a Wi-Fi device in the VM.
  netlinkProbe = pkgs.runCommandCC "whereami-netlink-generic-probe" { } ''
    mkdir -p "$out/bin"
    cat > probe.c <<'EOF'
#include <errno.h>
#include <linux/netlink.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

int main(void) {
  int fd = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_GENERIC);
  if (fd < 0) {
    perror("socket(AF_NETLINK, NETLINK_GENERIC)");
    return errno == 0 ? 1 : errno;
  }
  return close(fd) == 0 ? 0 : 1;
}
EOF
    $CC -O2 -Wall -Wextra -Werror probe.c -o "$out/bin/netlink-generic-probe"
  '';

  # Test-only command overlays. The daemon still runs its production scan loop,
  # spawns the packaged nmcli/iw paths, parses their real wire output, and
  # publishes through the real control socket. BindReadOnlyPaths below replaces
  # only those external binaries inside whereamid.service's mount namespace.
  mockNmcli = pkgs.writeShellScriptBin "nmcli" ''
    set -euo pipefail
    mode="$(${pkgs.coreutils}/bin/cat /run/whereami-scan-mode)"

    if (( $# == 10 )) \
      && [[ "$1" == "-t" && "$2" == "-f" \
            && "$3" == "BSSID,SSID,SIGNAL,CHAN,FREQ" \
            && "$4" == "device" && "$5" == "wifi" && "$6" == "list" \
            && "$7" == "ifname" && "$8" == "vmwifi0" \
            && "$9" == "--rescan" ]]; then
      [[ "''${LC_ALL-}" == "C" ]] || {
        echo "nmcli mock requires LC_ALL=C" >&2
        exit 65
      }
      rescan="''${10}"
      printf '%s\n' "$rescan" >> /var/lib/whereami/nmcli-lifecycle.log
      case "$mode:$rescan" in
        nmcli:yes)
          ${realNmcliProbe}/bin/nmcli-real --terse general status >/dev/null
          printf '%s\n' ok > /var/lib/whereami/nmcli-dbus-probe-ok
          ${pkgs.coreutils}/bin/sleep 0.01
          ${pkgs.gawk}/bin/awk '{ printf "%d\n", int($1 * 1000) }' \
            /proc/uptime > /run/whereami-last-scan
          # Poison the forced command's rows. Production must discard these
          # and publish only the post-generation --rescan no list below.
          printf '%s\n' '02\:00\:00\:00\:21\:99:POISON-forced-output:10:1:2412 MHz'
          exit 0
          ;;
        nmcli:no)
          printf '%s\n' '02\:00\:00\:00\:21\:10:VM-nmcli:80:6:2437 MHz'
          exit 0
          ;;
        iw:yes)
          echo "VM test forcing nmcli refresh failure for iw fallback" >&2
          exit 1
          ;;
        iw:no)
          # A successful but empty cached list is unconfirmed and must be the
          # only nmcli result allowed to fall through to blocking iw scan.
          exit 0
          ;;
        *)
          echo "unexpected nmcli lifecycle: $mode:$rescan" >&2
          exit 66
          ;;
      esac
    fi

    echo "unexpected nmcli arguments: $*" >&2
    exit 64
  '';

  mockBusctl = pkgs.writeShellScriptBin "busctl" ''
    set -euo pipefail
    printf '%s\n' "$*" >> /var/lib/whereami/busctl-lifecycle.log

    if (( $# == 9 )) \
      && [[ "$1" == "--json=short" && "$2" == "--system" \
            && "$3" == "call" && "$4" == "org.freedesktop.DBus" \
            && "$5" == "/org/freedesktop/DBus" \
            && "$6" == "org.freedesktop.DBus" && "$7" == "GetNameOwner" \
            && "$8" == "s" && "$9" == "org.freedesktop.NetworkManager" ]]; then
      printf '%s\n' '{"type":"s","data":[":1.4242"]}'
      exit 0
    fi

    if (( $# == 9 )) \
      && [[ "$1" == "--json=short" && "$2" == "--system" \
            && "$3" == "call" && "$4" == ":1.4242" \
            && "$5" == "/org/freedesktop/NetworkManager" \
            && "$6" == "org.freedesktop.NetworkManager" \
            && "$7" == "GetDeviceByIpIface" && "$8" == "s" \
            && "$9" == "vmwifi0" ]]; then
      printf '%s\n' '{"type":"o","data":["/org/freedesktop/NetworkManager/Devices/42"]}'
      exit 0
    fi

    if (( $# == 7 )) \
      && [[ "$1" == "--json=short" && "$2" == "--system" \
            && "$3" == "get-property" \
            && "$4" == ":1.4242" \
            && "$5" == "/org/freedesktop/NetworkManager/Devices/42" \
            && "$6" == "org.freedesktop.NetworkManager.Device.Wireless" \
            && "$7" == "LastScan" ]]; then
      value="$(${pkgs.coreutils}/bin/cat /run/whereami-last-scan)"
      printf '{"type":"x","data":%s}\n' "$value"
      exit 0
    fi

    echo "unexpected busctl arguments: $*" >&2
    exit 64
  '';

  mockIw = pkgs.writeShellScriptBin "iw" ''
    set -euo pipefail
    mode="$(${pkgs.coreutils}/bin/cat /run/whereami-scan-mode)"
    if [[ "$mode" != "iw" ]]; then
      echo "poisoned iw invoked during $mode phase" >&2
      exit 90
    fi
    if (( $# == 3 )) \
      && [[ "$1" == "dev" && "$2" == "vmwifi0" && "$3" == "scan" ]]; then
      operation=direct
    elif (( $# == 4 )) \
      && [[ "$1" == "dev" && "$2" == "vmwifi0" && "$3" == "scan" \
            && "$4" == "dump" ]]; then
      operation=dump
    else
      echo "unexpected iw arguments: $*" >&2
      exit 64
    fi

    # Read the child PID explicitly (not /proc/self) and fail unless the only
    # effective capability is CAP_NET_ADMIN (bit 12, exactly 0x1000).
    cap_hex="$(${pkgs.gawk}/bin/awk '$1 == "CapEff:" { print $2 }' /proc/$$/status)"
    if [[ "''${cap_hex,,}" != "0000000000001000" ]]; then
      echo "CAP_NET_ADMIN is not the exact effective set (CapEff=$cap_hex)" >&2
      exit 77
    fi
    ${netlinkProbe}/bin/netlink-generic-probe
    printf '%s %s\n' "$operation" "''${cap_hex,,}" >> /var/lib/whereami/iw-capeff.log

    case "$operation" in
      direct|dump)
        ${pkgs.coreutils}/bin/cat <<'EOF'
BSS 02:00:00:00:21:20(on vmwifi0)
        freq: 5180
        signal: -61.00 dBm
        SSID: VM-iw
        DS Parameter set: channel 36
EOF
        exit 0
        ;;
    esac

  '';

  pre0210RemovedSettings = [
    "PrivateDevices"
    "RestrictSUIDSGID"
    "ProtectProc"
    "ProcSubset"
    "ProtectHostname"
    "ProtectClock"
    "SystemCallArchitectures"
  ];
in
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
        # task-0206: pin one of the six newly-exposed flags end-to-end (option ->
        # exec-start.nix -> unit ExecStart), asserted below via `systemctl cat`.
        historyRetentionDays = 7;
        apsRetentionDays = 11;
        rustLog = "whereamid=debug";
        # task-0205: a 0600 root:root secrets file the DynamicUser must reach only
        # via LoadCredential (the module wires --config to $CREDENTIALS_DIRECTORY).
        credentialsFile = "/etc/whereami/secrets.toml";
      };
      # task-0205: 0600 root:root secrets with DUMMY WiGLE creds (NEVER real).
      # environment.etc routes `text` through the world-readable nix store, which
      # is acceptable ONLY because the values are dummies — a real deployment must
      # use an out-of-store 0600 file. The /etc file itself is a 0600 root:root
      # copy, which is what the daemon must be unable to read without LoadCredential.
      environment.etc."whereami/secrets.toml" = {
        mode = "0600";
        user = "root";
        group = "root";
        text = ''
          [wigle]
          api_user = "AID-dummy-not-a-real-credential"
          api_key = "dummy-not-a-real-key"
        '';
      };
      # sqlite is test tooling. The whereami package itself must come from the
      # production module's environment.systemPackages contract (TASK-0219).
      environment.systemPackages = [ pkgs.sqlite ];
      users.users.alice = { isNormalUser = true; };
      users.users.mallory = { isNormalUser = true; };

      # Measured pre-0210 control: clone the evaluated production service
      # configuration and remove only this task's seven new directives. It is
      # loaded but never started; systemd-analyze compares like-for-like units.
      systemd.services."whereamid-pre0210" = {
        description = "whereami pre-TASK-0210 security baseline (VM test only)";
        serviceConfig = builtins.removeAttrs
          config.systemd.services.whereamid.serviceConfig
          pre0210RemovedSettings;
      };
    };

    # Dedicated scanner node: test overlays cannot improve or otherwise alter
    # the production-shaped main node's systemd-analyze score.
    scan = { config, pkgs, ... }: {
      imports = [ self.nixosModules.default ];
      networking.networkmanager.enable = true;
      services.whereami = {
        enable = true;
        wifiInterface = "vmwifi0";
        scanIntervalFast = 1;
        scanFastDuration = 3600;
        scanIntervalSlow = 1;
        debounceWindow = 1;
        debounceThreshold = 1;
      };
      systemd.tmpfiles.rules = [
        "f /run/whereami-scan-mode 0644 root root - nmcli"
        "f /run/whereami-last-scan 0666 root root - 1"
      ];
      systemd.services.whereamid = {
        after = [ "systemd-tmpfiles-setup.service" ];
        serviceConfig.BindReadOnlyPaths = [
          "${mockNmcli}/bin/nmcli:${pkgs.networkmanager}/bin/nmcli"
          "${mockBusctl}/bin/busctl:${pkgs.systemd}/bin/busctl"
          "${mockIw}/bin/iw:${pkgs.iw}/bin/iw"
        ];
      };
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
    import json
    import shlex

    hardening_properties = [
        "PrivateDevices",
        "RestrictSUIDSGID",
        "ProtectProc",
        "ProcSubset",
        "ProtectHostname",
        "ProtectClock",
        "SystemCallArchitectures",
        "SystemCallFilter",
        "RestrictAddressFamilies",
        "AmbientCapabilities",
        "CapabilityBoundingSet",
    ]

    def effective_properties(node, unit="whereamid.service"):
        flags = " ".join(f"--property={name}" for name in hardening_properties)
        output = node.succeed(f"systemctl show {unit} {flags}")
        return dict(line.split("=", 1) for line in output.splitlines() if "=" in line)

    def assert_effective_hardening(node, label):
        props = effective_properties(node)
        evidence = dict(props)
        filter_count = len(props.get("SystemCallFilter", "").split())
        evidence["SystemCallFilter"] = f"{filter_count} allowed syscalls"
        print(
            f"TASK-0210 {label} effective hardening: "
            f"{json.dumps(evidence, sort_keys=True)}"
        )
        expected = {
            "PrivateDevices": "yes",
            "RestrictSUIDSGID": "yes",
            "ProtectProc": "invisible",
            "ProcSubset": "pid",
            "ProtectHostname": "yes",
            "ProtectClock": "yes",
            # The unit text below proves the declarative value; systemd exposes
            # the same canonical spelling through the runtime D-Bus property.
            "SystemCallArchitectures": "native",
        }
        for name, value in expected.items():
            assert props.get(name) == value, \
                f"{label}: {name}={props.get(name)!r}, expected {value!r}"
        assert props.get("SystemCallFilter"), f"{label}: SystemCallFilter is empty"
        assert set(props.get("RestrictAddressFamilies", "").split()) == {
            "AF_INET", "AF_INET6", "AF_NETLINK", "AF_UNIX"
        }, f"{label}: unexpected address families {props.get('RestrictAddressFamilies')!r}"
        assert props.get("AmbientCapabilities") == "cap_net_admin", \
            f"{label}: ambient capabilities drifted: {props.get('AmbientCapabilities')!r}"
        assert props.get("CapabilityBoundingSet") == "cap_net_admin", \
            f"{label}: capability bounding set drifted: {props.get('CapabilityBoundingSet')!r}"
        node.succeed(
            "systemctl cat whereamid.service | grep -F 'SystemCallArchitectures=native'"
        )
        node.succeed(
            "systemctl cat whereamid.service | grep -Fx 'RestrictSUIDSGID=true'"
        )
        node.succeed(
            "systemctl cat whereamid.service | grep -Fx 'SystemCallFilter=@system-service'"
        )
        node.succeed(
            "systemctl cat whereamid.service | grep -Fx 'SystemCallFilter=@network-io'"
        )
        return props

    def security_rows(node, unit):
        return json.loads(node.succeed(
            f"systemd-analyze security --json=short {unit}"
        ))

    def summed_exposure(rows):
        return sum(float(row["exposure"]) for row in rows if row["exposure"] is not None)

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

    with subtest("task-0219: module installs CLI and applies operator service controls"):
        # The node no longer adds cfg.package itself. A working command proves
        # services.whereami.enable supplies the operator CLI on the system PATH.
        cli = machine.succeed("command -v whereami").strip()
        assert cli == "/run/current-system/sw/bin/whereami", \
            f"module-provided whereami CLI missing from canonical PATH: {cli!r}"
        machine.succeed(
            "systemctl show whereamid.service -p Environment --value | "
            "grep -F 'RUST_LOG=whereamid=debug'"
        )
        restart = machine.succeed(
            "systemctl show whereamid.service -p Restart -p StartLimitBurst "
            "-p StartLimitIntervalUSec"
        )
        props = dict(line.split("=", 1) for line in restart.strip().splitlines())
        assert props == {
            "Restart": "on-failure",
            "StartLimitBurst": "5",
            "StartLimitIntervalUSec": "1min",
        }, f"operator restart contract drifted: {props!r}"

    with subtest("(e) non-loopback bind without allowNonlocalBind refuses to start"):
        # Check this intentionally failing node before its preserved restart
        # policy exhausts StartLimitBurst and propagates failure to the socket.
        # The daemon must reject the bind during validate(), before adopting fd 3.
        nonloopback.wait_for_unit("whereamid.socket")
        nonloopback.wait_until_succeeds("systemctl is-failed --quiet whereamid.service")
        nonloopback.succeed(
            "journalctl -u whereamid.service --no-pager | grep -F -- '--allow-nonlocal-bind'"
        )

    with subtest("task-0210: effective hardening and measured exposure improve"):
        main_hardening = assert_effective_hardening(machine, "main node")
        baseline_hardening = effective_properties(machine, "whereamid-pre0210.service")
        assert main_hardening["SystemCallFilter"] == baseline_hardening["SystemCallFilter"], \
            "TASK-0210 unexpectedly changed the effective syscall allowlist"
        hardened_rows = security_rows(machine, "whereamid.service")
        baseline_rows = security_rows(machine, "whereamid-pre0210.service")
        hardened_exposure = summed_exposure(hardened_rows)
        baseline_exposure = summed_exposure(baseline_rows)
        print(
            f"TASK-0210 systemd-analyze summed exposure: "
            f"{hardened_exposure:.3f} (hardened) vs {baseline_exposure:.3f} (pre-0210)"
        )
        assert hardened_exposure < baseline_exposure, \
            f"hardening did not improve exposure: {hardened_exposure} >= {baseline_exposure}"
        assert hardened_exposure < 3.0, \
            f"hardened summed exposure {hardened_exposure} exceeds ceiling 3.0"
        hardened_by_field = {row["json_field"]: row for row in hardened_rows}
        for field in [
            "PrivateDevices", "RestrictSUIDSGID", "ProtectProc", "ProcSubset",
            "ProtectHostname", "ProtectClock", "SystemCallArchitectures",
        ]:
            assert hardened_by_field[field]["set"] is True, \
                f"systemd-analyze does not recognize {field} as hardened"

    with subtest("task-0210: scan node has identical effective hardening"):
        scan.wait_for_unit("NetworkManager.service")
        scan.wait_for_unit("whereamid.socket")
        scan.wait_for_unit("whereamid.service")
        scan.wait_until_succeeds(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'control channel: unix socket ready'",
            timeout=30,
        )
        scan.succeed("test \"$(cat /run/whereami-scan-mode)\" = nmcli")
        scan_hardening = assert_effective_hardening(scan, "scan node")
        assert scan_hardening["SystemCallFilter"] == main_hardening["SystemCallFilter"], \
            "scan overlay changed the effective syscall allowlist"
        scan_pid = scan.succeed(
            "systemctl show -p MainPID --value whereamid.service"
        ).strip()
        scan_restarts = int(scan.succeed(
            "systemctl show -p NRestarts --value whereamid.service"
        ).strip())
        assert scan_pid != "0", "scan daemon has no live MainPID"
        assert scan_restarts == 0, f"scan daemon already restarted {scan_restarts} times"

    with subtest("task-0210: real nmcli DBus path and parser succeed under sandbox"):
        scan_query = (
            "env WHEREAMI_SOCKET=/run/whereami/whereamid.sock whereami scan --json"
        )
        scan.wait_until_succeeds(
            f"{scan_query} | grep -F '02:00:00:00:21:10'",
            timeout=30,
        )
        nmcli_scan = json.loads(scan.succeed(scan_query))
        nmcli_networks = nmcli_scan["networks"]
        assert len(nmcli_networks) == 1, f"unexpected nmcli scan: {nmcli_scan!r}"
        assert nmcli_networks[0] == {
            "bssid": "02:00:00:00:21:10",
            "ssid": "VM-nmcli",
            "signal_dbm": -42,
            "channel": 6,
        }, f"nmcli parser/wire response drifted: {nmcli_scan!r}"
        scan.succeed("test -f /var/lib/whereami/nmcli-dbus-probe-ok")
        scan.succeed(
            "grep -F 'GetNameOwner s org.freedesktop.NetworkManager' "
            "/var/lib/whereami/busctl-lifecycle.log"
        )
        scan.succeed(
            "grep -F 'call :1.4242 /org/freedesktop/NetworkManager "
            "org.freedesktop.NetworkManager GetDeviceByIpIface s vmwifi0' "
            "/var/lib/whereami/busctl-lifecycle.log"
        )
        scan.succeed(
            "grep -F 'get-property :1.4242 "
            "/org/freedesktop/NetworkManager/Devices/42 "
            "org.freedesktop.NetworkManager.Device.Wireless LastScan' "
            "/var/lib/whereami/busctl-lifecycle.log"
        )
        scan.succeed("test ! -e /var/lib/whereami/iw-capeff.log")
        scan.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'poisoned iw invoked'"
        )
        assert scan.succeed(
            "systemctl show -p MainPID --value whereamid.service"
        ).strip() == scan_pid, "nmcli scan restarted the daemon"
        assert int(scan.succeed(
            "systemctl show -p NRestarts --value whereamid.service"
        ).strip()) == 0, "nmcli scan incremented NRestarts"

    with subtest("task-0210: iw capability, netlink path and parser succeed under sandbox"):
        # Atomic, deterministic phase transition. nmcli now fails closed and
        # the next production scan-loop iteration must take the iw fallback.
        scan.succeed(
            "printf '%s\\n' iw > /run/whereami-scan-mode.next && "
            "mv /run/whereami-scan-mode.next /run/whereami-scan-mode"
        )
        scan.wait_until_succeeds(
            f"{scan_query} | grep -F '02:00:00:00:21:20'",
            timeout=30,
        )
        iw_scan = json.loads(scan.succeed(scan_query))
        iw_networks = iw_scan["networks"]
        assert len(iw_networks) == 1, f"unexpected iw scan: {iw_scan!r}"
        assert iw_networks[0] == {
            "bssid": "02:00:00:00:21:20",
            "ssid": "VM-iw",
            "signal_dbm": -61,
            "channel": 36,
        }, f"iw parser/wire response drifted: {iw_scan!r}"
        assert all(n["bssid"] != "02:00:00:00:21:10" for n in iw_networks), \
            f"stale nmcli result survived deterministic switch: {iw_scan!r}"
        scan.succeed(
            "grep -Fx 'direct 0000000000001000' /var/lib/whereami/iw-capeff.log"
        )
        scan.fail(
            "grep -F 'dump ' /var/lib/whereami/iw-capeff.log"
        )
        scan.succeed("grep -Fx yes /var/lib/whereami/nmcli-lifecycle.log")
        scan.succeed("grep -Fx no /var/lib/whereami/nmcli-lifecycle.log")
        assert scan.succeed(
            "systemctl show -p MainPID --value whereamid.service"
        ).strip() == scan_pid, "iw scan restarted the daemon"
        assert int(scan.succeed(
            "systemctl show -p NRestarts --value whereamid.service"
        ).strip()) == 0, "iw scan incremented NRestarts"
        scan.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -E 'poisoned iw|unexpected (nmcli|iw|busctl) arguments|"
            "CAP_NET_ADMIN is not the exact|Operation not permitted|"
            "Permission denied|Bad system call|NETLINK_GENERIC'"
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
        alice_uid = machine.succeed("id -u alice").strip()
        machine.wait_until_succeeds(
            "journalctl -u whereamid.service --no-pager | "
            f"grep -F 'unix connection uid={alice_uid} '"
        )

    with subtest("task-0206: a newly-exposed flag reaches the unit ExecStart"):
        # historyRetentionDays=7 on the node must appear as --history-retention-days
        # 7 in the generated unit. Cheap end-to-end pin of option -> exec-start ->
        # ExecStart for the six flags added in task-0206.
        machine.succeed(
            "systemctl cat whereamid.service | grep -F -- '--history-retention-days 7'"
        )
        machine.succeed(
            "systemctl cat whereamid.service | grep -F -- '--aps-retention-days 11'"
        )

    with subtest("task-0205: WiGLE secrets load via LoadCredential without widening the source"):
        # AC#1/#3: the 0600 root:root file reaches the DynamicUser through systemd
        # credentials, so the daemon finds it (NOT the "config file not found"
        # warning) and reports WiGLE configured. A broken credentials-directory
        # expansion would surface as the not-found warning, failing this subtest.
        machine.fail(
            "journalctl -u whereamid.service --no-pager | grep -F 'config file not found'"
        )
        machine.succeed(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'WiGLE credentials configured'"
        )
        machine.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'WiGLE credentials not configured'"
        )
        # systemd's protected managed copy is 0440 on this image. The daemon
        # must recognize only that exact $CREDENTIALS_DIRECTORY/config path,
        # rather than emitting its generic warning for an ordinary loose 0440
        # file. Unit tests pin the ordinary and near-miss cases.
        machine.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'WiGLE credentials file' | grep -F 'permissions'"
        )
        machine.fail(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'another local user'"
        )
        # AC#1/#2: LoadCredential COPIES the source; the original must stay
        # 0600 root:root (nothing widened it to make it world-readable).
        mode = machine.succeed("stat -c %a /etc/whereami/secrets.toml").strip()
        owner = machine.succeed("stat -c %U:%G /etc/whereami/secrets.toml").strip()
        assert mode == "600", f"secrets file mode is {mode}, expected 600 (unwidened)"
        assert owner == "root:root", f"secrets file owner is {owner}, expected root:root"

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

    with subtest("task-0202: confirmed Unix wipe deletes live and persisted privacy state"):
        db = "/var/lib/whereami/aps.sqlite"

        def sqlite_value(sql):
            return machine.succeed(
                f"sqlite3 -batch -noheader {shlex.quote(db)} {shlex.quote(sql)}"
            ).strip()

        seed_sql = """
          BEGIN IMMEDIATE;
          INSERT INTO aps (
            bssid, ssid, lat, lon, source, source_priority,
            first_seen, last_seen, fetched_at
          ) VALUES (
            '02:00:00:00:20:20', 'wipe-private-ssid', 55.123456, 12.654321,
            'manual', 10, '2999-01-01T00:00:00+00:00',
            '2999-01-01T00:00:00+00:00', '2999-01-01T00:00:00+00:00'
          );
          INSERT INTO pending (
            bssid, ssid, first_seen, last_seen, attempts
          ) VALUES (
            '02:00:00:00:20:21', 'wipe-pending-ssid',
            '2999-01-01T00:00:00+00:00', '2999-01-01T00:00:00+00:00', 0
          );
          INSERT INTO not_found (
            bssid, first_seen, last_seen, checked_at
          ) VALUES (
            '02:00:00:00:20:22', '2999-01-01T00:00:00+00:00',
            '2999-01-01T00:00:00+00:00', '2999-01-01T00:00:00+00:00'
          );
          INSERT INTO fixes (
            at_rfc3339, lat, lon, accuracy_m, n_sources
          ) VALUES ('2999-01-01T00:00:00+00:00', 55.123456, 12.654321, 4.0, 1);
          INSERT INTO last_fix (
            id, lat, lon, accuracy_m, address, at_rfc3339, sources
          ) VALUES (
            1, 55.123456, 12.654321, 4.0, 'wipe-private-address',
            '2999-01-01T00:00:00+00:00', 1
          );
          INSERT INTO metadata (key, value) VALUES ('wipe-vm-retained', 'yes');
          COMMIT;
        """
        machine.succeed(
            f"sqlite3 -cmd '.timeout 5000' -batch {shlex.quote(db)} {shlex.quote(seed_sql)}"
        )
        assert sqlite_value(
            "SELECT (SELECT count(*) FROM aps) || ',' || "
            "(SELECT count(*) FROM pending) || ',' || "
            "(SELECT count(*) FROM not_found) || ',' || "
            "(SELECT count(*) FROM fixes) || ',' || "
            "(SELECT count(*) FROM last_fix);"
        ) == "1,1,1,1,1"

        # Restart forces last_fix through the production DB rehydration path;
        # locate then proves the sensitive value is live in managed memory.
        machine.succeed("systemctl restart whereamid.service")
        machine.wait_for_unit("whereamid.service")
        locate = machine.wait_until_succeeds(
            "runuser -u alice -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami --json locate | grep -F '55.123456'",
            timeout=30,
        )
        assert "55.123456" in locate

        tcp_refusal = machine.fail(
            "env WHEREAMI_ADDR=127.0.0.1:4747 whereami --json wipe --yes"
        )
        assert "unavailable over unauthenticated TCP" in tcp_refusal, tcp_refusal
        assert sqlite_value("SELECT count(*) FROM aps;") == "1", \
            "TCP refusal must not delete state"

        member_wipe = json.loads(machine.succeed(
            "runuser -u alice -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami --json wipe --yes"
        ))
        assert member_wipe == {
            "ok": True,
            "v": 1,
            "aps_deleted": 1,
            "pending_deleted": 1,
            "not_found_deleted": 1,
            "fixes_deleted": 1,
            "last_fix_deleted": 1,
        }, f"unexpected allowedUsers-member wipe response: {member_wipe!r}"

        # Keep the documented root form covered independently. Wipe is
        # idempotent, so the already-empty response must be an exact success.
        root_wipe = json.loads(machine.succeed(
            "env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami --json wipe --yes"
        ))
        assert root_wipe == {
            "ok": True,
            "v": 1,
            "aps_deleted": 0,
            "pending_deleted": 0,
            "not_found_deleted": 0,
            "fixes_deleted": 0,
            "last_fix_deleted": 0,
        }, f"unexpected idempotent root wipe response: {root_wipe!r}"
        assert sqlite_value(
            "SELECT (SELECT count(*) FROM aps) || ',' || "
            "(SELECT count(*) FROM pending) || ',' || "
            "(SELECT count(*) FROM not_found) || ',' || "
            "(SELECT count(*) FROM fixes) || ',' || "
            "(SELECT count(*) FROM last_fix) || ',' || "
            "(SELECT count(*) FROM sqlite_sequence WHERE name = 'fixes');"
        ) == "0,0,0,0,0,0"
        assert sqlite_value(
            "SELECT value FROM metadata WHERE key = 'wipe-vm-retained';"
        ) == "yes", "wipe must retain operational metadata"
        # Privacy wipe retains the current operational schema marker. This
        # fixture predates the v6 mobile-AP migration and must track the schema
        # version the production daemon actually initialized above.
        assert sqlite_value("SELECT version FROM schema_version WHERE id = 1;") == "6"

        after = machine.fail(
            "runuser -u alice -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami --json locate"
        )
        assert '"ok":false' in after.replace(" ", ""), after
        assert "55.123456" not in after, "wiped in-memory last_fix leaked after wipe response"

    with subtest("task-0204: SIGKILL triggers auto-restart and the daemon answers again"):
        # AC#1/#2: hard-kill the running daemon and confirm Restart=on-failure
        # brings it back with a NEW main PID that still answers on the socket.
        # A restart that leaves a broken/silent socket is a failure, so we assert
        # a live version response from the restarted instance (not just active).
        old_pid = machine.succeed(
            "systemctl show -p MainPID --value whereamid.service"
        ).strip()
        machine.succeed("systemctl kill --signal=SIGKILL whereamid.service")
        # New instance == MainPID becomes nonzero AND differs from the killed one.
        machine.wait_until_succeeds(
            "p=$(systemctl show -p MainPID --value whereamid.service); "
            f'[ "$p" != 0 ] && [ "$p" != {old_pid} ]'
        )
        machine.wait_for_unit("whereamid.service")
        # Settle wait only: this grep also matches the PRE-kill ready line in the
        # unit's journal history, so it does NOT prove the restarted instance is
        # ready — the live query below is the actual proof of re-adoption.
        machine.wait_until_succeeds(
            "journalctl -u whereamid.service --no-pager | "
            "grep -F 'control channel: unix socket ready'"
        )
        out = machine.succeed(
            "runuser -u alice -- env WHEREAMI_SOCKET=/run/whereami/whereamid.sock "
            "whereami version --json"
        )
        assert '"ok":true' in out.replace(" ", ""), \
            f"restarted daemon must answer alice, got: {out!r}"

  '';
}
