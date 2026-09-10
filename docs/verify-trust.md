# Verify it yourself

Baton is closed-source, so you should not have to take its word for anything. Three facts
about it can be checked by you, on your own computer:

1. **It communicates only locally.** Baton's own code makes no network connection. Checked
   below.
2. **Nothing leaves without a preview.** Every screenshot is shown to you, with the secrets
   hidden, before you decide what to send (see [Screenshots and privacy](screenshots-and-privacy.md)).
3. **Everything sent is recorded on this computer.** The log keeps the text of every send,
   and for an image its size, hash and hidden areas (see [Log and export](log-and-export.md)).

## What Baton talks to

- **The MCP server on this computer**, `handoff-mcp.exe`, which your agent starts. The two
  speak over a named pipe (`\\.\pipe\handoff-…`) that only your Windows account may open, and
  the server must also present the token in `%USERPROFILE%\.handoff\channel.token`, which only
  your account can read. The token keeps out other users of this computer and accidental
  connections; it does not protect against a malicious program already running as you — that
  is Windows' boundary. The channel is described in the server's documentation
  ([the internal channel](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/channel.md)).
- **Your agent**, through that server, which talks to it over its standard input and output.
  What you send to the agent, the agent sends on to its model provider as part of the
  conversation, like anything you type into it.

Baton's code talks to nothing else. It contains exactly one place that could open a network
connection, kept for a future update check; in this build nothing calls it, and the build does
not even include an HTTP client. The page inside Baton's window is not allowed to open
connections either: its content security policy is `default-src 'self'`, with no
`connect-src`.

## The window is drawn by Microsoft Edge WebView2

Baton's panel is a web page drawn by **Microsoft Edge WebView2**, a component of Windows. It
runs as separate programs, `msedgewebview2.exe`, which Baton starts and which stop when Baton
does. WebView2 makes network connections of its own, for its own purposes, that Baton's code
does not ask for and cannot see.

Baton starts WebView2 with these switches, which turn off its background networking:
`--disable-background-networking`, `--disable-component-update`,
`--disable-domain-reliability`, `--no-pings` (plus the three features wry, the library Baton
uses to embed WebView2, already turns off: `msWebOOUI`, `msPdfOOUI`, `msSmartScreenProtection`).

### What remains: measured

Measured on 10 September 2026 on Windows 11 (build 26200), WebView2 Runtime 152.0.4191.66,
with the command below, watching Baton and every process it started for 70 seconds from
launch:

| Process | Without the switches | With the switches (this build) |
|---|---|---|
| `handoff-app.exe` (Baton) | no connection | no connection |
| `handoff-mcp.exe` (the server) | no connection | no connection |
| WebView2 network service | an HTTPS connection to a Microsoft address about 60 seconds after launch, and a UDP socket | nothing |
| WebView2 browser process | one HTTPS connection to a Microsoft address within 5 seconds of launch | **one HTTPS connection to a Microsoft address within 4 seconds of launch** |

So **one connection remains, and it is WebView2's own**:

- **Who opens it:** the WebView2 browser process — the `msedgewebview2.exe` that is a direct
  child of `handoff-app.exe`. Not Baton's code, and not the page: the page's own requests would
  go through WebView2's network service, which the policy above forbids and which made none.
  It is outside the part of WebView2 the switches control, which is why they do not stop it.
- **When:** within a few seconds of Baton starting, with or without a handoff open, with the
  panel hidden or shown. It is a single HTTPS connection (port 443) that stays open.
- **Where to:** a different address from one launch to the next; measured were `40.101.113.8`,
  `40.101.113.65`, `52.97.135.50` and `52.97.232.194` (and, before the
  switches, `40.99.217.34`, `40.101.22.192`, `52.97.201.226`, `52.98.237.146` and
  `150.171.28.11`), all in address blocks that belong to Microsoft. None of them has a reverse
  DNS name and Windows' DNS cache held no name for them, so this page cannot tell you which
  Microsoft service it is. The connection is encrypted; Baton cannot see what it carries.

What WebView2 does is governed by Microsoft, not by Baton. If you want to see it for
yourself, the command below shows it.

## See every connection yourself

Paste this into a PowerShell window — no administrator rights are needed — and leave it
running while you use Baton. It follows Baton's two programs and **every process they start**,
WebView2 included, and prints each network connection one of them makes the first time it
appears. `Ctrl+C` stops it.

```powershell
# Baton network watch: every connection of Baton and of each process it starts.
# Leave it running while you use Baton; Ctrl+C stops it.
$roots = @('handoff-app.exe', 'handoff-mcp.exe')
$seen = @{}
$last = ''
while ($true) {
  $all = @(Get-CimInstance Win32_Process)
  $tree = @{}
  foreach ($p in $all) { if ($roots -contains $p.Name) { $tree[[int]$p.ProcessId] = $p.Name } }
  do {
    $grew = $false
    foreach ($p in $all) {
      $id = [int]$p.ProcessId
      if ($id -ne 0 -and -not $tree.ContainsKey($id) -and $tree.ContainsKey([int]$p.ParentProcessId)) {
        $tree[$id] = $p.Name
        $grew = $true
      }
    }
  } while ($grew)
  $now = ($tree.Keys | Sort-Object | ForEach-Object { "$($tree[$_]) $_" }) -join ', '
  if ($now -ne $last) {
    $last = $now
    if ($now -eq '') { $now = 'no Baton process is running' }
    '{0:HH:mm:ss}  watching: {1}' -f (Get-Date), $now
  }
  Get-NetTCPConnection -ErrorAction SilentlyContinue |
    Where-Object { $tree.ContainsKey([int]$_.OwningProcess) -and $_.State -notin 'Listen', 'Bound' -and $_.RemoteAddress -notmatch '^(127\.|::1$|0\.0\.0\.0$|::$)' } |
    ForEach-Object {
      $key = "tcp $($_.OwningProcess) $($_.RemoteAddress) $($_.RemotePort)"
      if (-not $seen.ContainsKey($key)) {
        $seen[$key] = $true
        '{0:HH:mm:ss}  TCP  {1} (pid {2}) -> {3} port {4}, {5}' -f (Get-Date), $tree[[int]$_.OwningProcess], $_.OwningProcess, $_.RemoteAddress, $_.RemotePort, $_.State
      }
    }
  Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
    Where-Object { $tree.ContainsKey([int]$_.OwningProcess) -and $_.LocalAddress -notmatch '^(127\.|::1$)' } |
    ForEach-Object {
      $key = "udp $($_.OwningProcess) $($_.LocalPort)"
      if (-not $seen.ContainsKey($key)) {
        $seen[$key] = $true
        '{0:HH:mm:ss}  UDP  {1} (pid {2}) opened a socket on port {3}' -f (Get-Date), $tree[[int]$_.OwningProcess], $_.OwningProcess, $_.LocalPort
      }
    }
  Start-Sleep -Milliseconds 500
}
```

What to expect in this build:

- a `watching:` line listing `handoff-app.exe`, the `msedgewebview2.exe` processes it started
  and any `handoff-mcp.exe` your agents are running;
- **one** line like `TCP  msedgewebview2.exe (pid 39120) -> 40.101.113.8 port 443, Established`:
  that is the WebView2 connection described above;
- **no line naming `handoff-app.exe` or `handoff-mcp.exe`**, however you use Baton. Such a line
  would be Baton's own code making a connection, which this build must never do.

The command leaves out connections to this computer itself (`127.0.0.1` and `::1`): WebView2
may briefly try one when it starts, and it never leaves the machine. A connection that opens
and closes within half a second can slip between two looks; the firewall test below does not
have that limit, for Baton's own programs.

## The firewall test

This test shows that Baton's own programs need no network at all: block them, use Baton
normally, and nothing fails.

1. Open PowerShell **as administrator**, as the same Windows user who installed Baton, and
   block both programs:

   ```powershell
   New-NetFirewallRule -DisplayName 'Block Baton' -Direction Outbound -Action Block -Program "$env:LOCALAPPDATA\Baton\handoff-app.exe"
   New-NetFirewallRule -DisplayName 'Block Baton server' -Direction Outbound -Action Block -Program "$env:LOCALAPPDATA\Baton\handoff-mcp.exe"
   ```

   (The same rules can be created by hand in *Windows Defender Firewall with Advanced
   Security* → *Outbound Rules* → *New Rule* → *Program*.)
2. Use Baton for a full handoff, with a screenshot sent both as an image and as text.
3. **Expected:** nothing fails, and Settings → Network in Baton still says *No connection has
   been recorded*. The watch command above prints no line for `handoff-app.exe` or
   `handoff-mcp.exe`.
4. Remove the rules:

   ```powershell
   Remove-NetFirewallRule -DisplayName 'Block Baton', 'Block Baton server'
   ```

The rules do not cover WebView2: it is a different program, `msedgewebview2.exe`, shared with
other applications on your computer, and blocking it would break them. That is why the watch
command follows the whole process tree instead.

In this build the expected number of connections from Baton's own programs is **zero**. A
future build with an update check will make exactly one, to one fixed domain, and this page
will name it then.

## The Network page

Settings → Network lists every connection Baton's own code makes, with its date, domain and
bytes sent — each one is recorded before anything is sent. In this build the list is always
empty, because nothing calls the one place that could connect. The page cannot list WebView2's
connections: they are not made by Baton's code. The page is what Baton says about itself; the
watch command and the firewall test are how you check it.

## What the developers check on every change

- The shipped build contains no HTTP client; a test fails if any part of Baton's code other
  than its single network module names a network type, and another if anything calls that
  module.
- A test fails if the window's content security policy gains a `connect-src`, or if a window
  starts WebView2 without the switches above.
- The end-to-end suite, which drives a real agent through every kind of handoff, fails if any
  connection is recorded on the Network page.
