# Fails if the app's dependency tree contains anything that can open a network connection.
# Run from anywhere; it is meant to run in CI on every change.
#
# The window is a WebView2 page, so the other half of the guarantee is the content security
# policy and the capability file in app/crates/umbra-app, which this script also checks.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root 'app'
Push-Location $app
try {
    $denied = @(
        'reqwest', 'hyper', 'hyper-util', 'ureq', 'curl', 'curl-sys', 'isahc', 'surf', 'attohttpc',
        'h2', 'h3', 'quinn', 'native-tls', 'rustls', 'openssl', 'openssl-sys', 'schannel',
        'socket2', 'mio', 'async-std', 'smol', 'tungstenite', 'tokio-tungstenite', 'trust-dns-resolver',
        'hickory-resolver', 'tauri-plugin-http', 'tauri-plugin-updater', 'tauri-plugin-shell',
        'tauri-plugin-websocket', 'tauri-plugin-upload', 'tauri-plugin-opener'
    )
    $tree = cargo tree -p umbra --prefix none --edges normal,build 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'cargo tree failed' }
    $names = $tree | ForEach-Object { ($_ -split ' ')[0] } | Sort-Object -Unique
    $found = $names | Where-Object { $denied -contains $_ }
    $problems = @()
    if ($found) { $problems += "Networking crates in the dependency tree: $($found -join ', ')" }

    # tokio may be present for Tauri's runtime, but must not have sockets.
    $tokioFeatures = cargo tree -p umbra -i tokio -e features 2>$null | Select-String -Pattern 'tokio feature "(net|process|signal)"'
    if ($tokioFeatures) { $problems += 'tokio has the "net" feature enabled' }

    $conf = Get-Content (Join-Path $app 'crates\umbra-app\tauri.conf.json') -Raw | ConvertFrom-Json
    $csp = $conf.app.security.csp
    if ($csp -match 'https?://(?!asset\.localhost|ipc\.localhost)') { $problems += "CSP allows a remote origin: $csp" }
    if ($csp -match "connect-src[^;]*(\*|https:|http:(?!//ipc))") { $problems += 'CSP connect-src is too open' }

    # std::net needs no crate. Only two listeners may use it, both on 127.0.0.1: the window's
    # media server and the CS2 auto-clip listener.
    $listeners = @{
        'umbra-app\src\media.rs' = 'TcpListener::bind\(\("127\.0\.0\.1", 0\)\)'
        'umbra-core\src\cs2.rs' = 'TcpListener::bind\(\(Ipv4Addr::LOCALHOST, port\)\)'
    }
    $sockets = Get-ChildItem crates -Recurse -Filter *.rs |
        Select-String -Pattern 'std::net|TcpStream|TcpListener|UdpSocket' |
        Where-Object { $path = $_.Path; -not ($listeners.Keys | Where-Object { $path -like "*\$_" }) }
    if ($sockets) { $problems += "Sockets outside the local listeners: $(($sockets | ForEach-Object { "$($_.Filename):$($_.LineNumber)" }) -join ', ')" }
    foreach ($file in $listeners.Keys) {
        $code = Get-Content (Join-Path 'crates' $file) -Raw
        if ($code -notmatch $listeners[$file] -or $code -match 'UNSPECIFIED|0\.0\.0\.0') {
            $problems += "$file must listen on 127.0.0.1 only"
        }
    }

    # Judge the permission identifiers, not the prose around them.
    $caps = Get-Content (Join-Path $app 'crates\umbra-app\capabilities\default.json') -Raw | ConvertFrom-Json
    $bad = $caps.permissions | Where-Object { $_ -match '^(http|shell|fs|opener|websocket|updater|upload|process|os|dialog):' }
    if ($bad) { $problems += "A capability grants network-capable access: $($bad -join ', ')" }

    if ($problems.Count -gt 0) {
        $problems | ForEach-Object { Write-Error $_ -ErrorAction Continue }
        exit 1
    }
    Write-Host "OK: no networking crates, no remote origins in the CSP, no network capabilities."
}
finally {
    Pop-Location
}
