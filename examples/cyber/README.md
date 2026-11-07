# Cyber investigation demo

One synthetic day of endpoint telemetry from a fictional company: who logged
into which machine, which processes ran there, where they connected and
which domains resolve to those addresses.

- Users and hostnames are fictional.
- IP addresses come from the documentation ranges of
  [RFC 5737](https://www.rfc-editor.org/rfc/rfc5737) (192.0.2.0/24,
  198.51.100.0/24, 203.0.113.0/24) and domains use the reserved `example`
  names of [RFC 2606](https://www.rfc-editor.org/rfc/rfc2606), so nothing
  points at a real system.
- **Nothing in this dataset is a confirmed incident or labelled malicious.**
  It is practice material for following connections.

```bash
make demo-cyber
```

Re-running reuses and verifies the graph `Demo: Cyber investigation`. Without
make: `docker compose --profile demo run --rm demo cyber`.

## The data

| File | Rows | Becomes |
|---|---|---|
| `logins.csv` | 6 | `User` and `Endpoint`, `User -LOGGED_INTO-> Endpoint` (logon type, time) |
| `processes.csv` | 6 | `Process`, `Endpoint -EXECUTED-> Process` |
| `connections.csv` | 6 | `IPAddress`, `Process -CONNECTED_TO-> IPAddress` (port, protocol, bytes sent, time) |
| `dns.csv` | 5 | `Domain`, `Domain -RESOLVED_TO-> IPAddress` |

## What the loader checks

| Check | Expected |
|---|---|
| Entities | 4 User, 4 Endpoint, 6 Process, 4 IPAddress, 5 Domain |
| Relationships created | 23 |
| WS-FIN-07 | 2 users logged into it (j.santoso interactively, r.wijaya remotely) |
| 203.0.113.45 | 2 processes connected to it; 2 domains resolve to it |
| Path j.santoso → update-check.example.net | 4 hops: `LOGGED_INTO · EXECUTED · CONNECTED_TO · RESOLVED_TO` |
| Path WS-FIN-07 → WS-FIN-12 | 4 hops: `EXECUTED · CONNECTED_TO · CONNECTED_TO · EXECUTED` |

## What to look at

1. Start on **WS-FIN-07**. Two users logged into it: Joko Santoso in the
   morning and Rina Wijaya (IT) remotely at 13:40.
2. Two minutes later the workstation ran `updater.exe` from a user's
   temporary folder, which connected to 203.0.113.45.
3. Find Path from WS-FIN-07 to **WS-FIN-12**: another Finance workstation
   ran a process with the same name from a temporary folder, connecting to
   the same address. Two domains, `update-check.example.net` and
   `files-sync.example.net`, resolve to it.

Whether that is a software updater, an IT tool or something to escalate is
exactly the question an analyst would investigate further. The graph shows
the connections; it does not decide.
