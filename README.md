# Stabilizer-NaNo

Open-source, ad-free Android mobile data stabilizer powered by a **Rust core (`rust_core`)** bridged via **UniFFI** and a **Jetpack Compose (Material 3)** Android UI/service layer.

## Features

- **Idle-aware state machine (no fixed timer spam):**
  - **Active:** Real mobile data traffic is moving → **0 pings**.
  - **IdlePing:** Mobile data idle for `idle_threshold_secs` (default `4s`) → sends **1 lightweight HTTP `HEAD` request** (`rustls-tls`) to `https://connectivitycheck.gstatic.com/generate_204` (or custom URL).
  - **Warmup:** Screen turns on or unlocks → sends **1 instant `HEAD` ping**.
  - **Sleeping:** Screen off with no background transfer, OR screen on in an offline app with no real non-ping traffic for `offline_sleep_secs` (default `120s`) → **0 pings** until real traffic or screen wake resumes.
  - **Recovering:** Loss of `NET_CAPABILITY_VALIDATED` while active → sends immediate wake-up ping; if still unvalidated after `10s` and Root Auto-Recovery is enabled, runs `su -c "svc data disable && sleep 1 && svc data enable"`.
- **Self-traffic exclusion:** Samples `TrafficStats.getMobileRxBytes()` / `getMobileTxBytes()` every `1s` in memory and subtracts this app's own `TrafficStats.getUidRxBytes()` / `getUidTxBytes()` so stabilizer pings never reset the idle timer.
- **Foreground Service & Quick Settings Tile:** Low-priority persistent notification and one-tap Quick Settings tile (`Net Stabilizer`).
- **Material You UI:** Live state badge, ping latency & traffic telemetry, live log feed, configurable thresholds/URL, and battery optimization exemption prompt.

## Repository Layout

- `rust_core/` — Tokio + Reqwest (`rustls-tls`) + UniFFI idle-aware state machine & unit tests
- `app/` — Kotlin + Jetpack Compose Material 3 Android app, `ForegroundService`, `TileService`, `NetworkCallback`, `BroadcastReceiver`, and `TrafficStats` sampler
- `.github/workflows/build.yml` — Automated cloud CI building `arm64-v8a` `librust_core.so`, UniFFI Kotlin bindings, and `app-debug.apk`

## Downloading the APK from GitHub Actions

1. Open the **Actions** tab in this GitHub repository.
2. Select the latest successful **Build Stabilizer-NaNo APK** workflow run.
3. Under **Artifacts**, download **`app-debug-apk`** (`app-debug.apk` inside the zip).

Or via GitHub CLI:
```bash
gh run list --workflow "Build Stabilizer-NaNo APK" --limit 5
gh run download <run-id> -n app-debug-apk
```

## Local Testing (Rust Core)

```bash
cd rust_core
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```
