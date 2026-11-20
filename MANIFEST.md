# PR #17977 Execution & Network Proof Bundle

- **PR**: `horizon-rust/horizon#17977` (`bug/horizon-brand-behind-velocity`)
- **Verified Commit (HEAD)**: `cca49a532af14b33e09207ceb6cb183d56890be2`
- **Platform**: Windows 11 (`x86_64`), Rust `stable` (`1.97+`)

## 1. Binary & Pinned Resource Identities (SHA-256)

| Artifact | Size (bytes) | SHA-256 |
| --- | ---: | --- |
| `horizon-client.exe` (`--profile fast`) | 159674880 | `8b1980b9d7ed2acef0bdb7443322952b66c4bb37c5e5c983de24c86efa44b1cf` |
| `horizon-ctl.exe` (`--profile fast`) | 1526272 | `32599eb361bdee7ace58f7f0d8fed798afc8b3636125eeaaae8676ebf929b6ba` |
| Official Minecraft `26.3` `server.jar` | 62294556 | `d052f14d7a173734fba553711e5b570162e2f2a313267ee31a21b975a679be64` |
| Official PaperMC Velocity `4.3.2-SNAPSHOT` (build 50) `velocity.jar` | 42392496 | `3d3934f3343743392d404d9faad6d70fb2ed1cce82833923dc63f96cca72df9a` |
| Populated `side = "both"` test mod (`probe_shared/horizon-mod.toml`) | 116 | `4874b043fc96586484ede219bddbdb894dec9f0189721e6cbed8bf75a367a603` |

## 2. Commands & Exit Codes

1. **Resource preparation (`26.3`)**:
   - `horizon-prepare.exe --prepare --complete --server-jar server-26.3.jar --root <WORKTREE>` -> **exit `0`** (12.7 s, 4962 sounds ready).
2. **Real-network wire capture (`direct_horizon`, `horizon_behind_velocity`, `vanilla_26_3`)**:
   - `python pr17977_capture.py <OUT>` -> **exit `0`** (`CAPTURED ALL 3 SCENARIOS SUCCESSFULLY`).
   - Client loaded `--dev-mod probe_shared` (`id = "probe_shared"`, `version = "0.1.0"`, `side = "both"`) so `horizon:mod_list` carried a non-empty mod list.
   - Full decoded packet trace: [`wire_capture.json`](wire_capture.json).
3. **`horizon-clients-only` server E2E**:
   - `python tools/server_horizon_only_e2e.py --bin-dir <BIN_DIR>` -> **exit `0`** (`off`: client without brand joins; `on`: no brand and `"vanilla"` rejected, `"horizon"` joins).
4. **LAN & Single-Player Host E2E (`tab`, `chest`, `chat`)**:
   - `python tools/lan_e2e.py --bin-dir <BIN_DIR> --check tab` -> **exit `0`** (`['Guest', 'Player59267']`).
   - `python tools/lan_e2e.py --bin-dir <BIN_DIR> --check chest` -> **exit `0`**.
   - `python tools/lan_e2e.py --bin-dir <BIN_DIR> --check chat` -> **exit `0`** (`{'guest': True, 'host': True}`).
   - Logs, `results.json`, and screenshots: [`lan_e2e/`](lan_e2e/).
5. **Single-player client smoke**:
   - `horizon-client.exe --resources <RESOURCES> --game-dir <DIR> --frames 60 --smoke-report report.json` -> **exit `0`** (`frames: 60`, `error: null`).
6. **Unit tests (`horizon-client-lib`)**:
   - `cargo +stable test -p horizon-client-lib --lib net::tests -- --nocapture` -> **exit `0`** (`58 passed; 0 failed; 3 ignored`).
7. **Parity harnesses (`tools/parity.py --hosts=""`)**:
   - `secure_chat`: **PASS** (`158s`) — [`parity/secure_chat.log`](parity/secure_chat.log)
   - `server_address`: **PASS** (`909s`) — [`parity/server_address.log`](parity/server_address.log)
   - `resource_pack`: **PASS** (`930s`) — [`parity/resource_pack.log`](parity/resource_pack.log)
   - (`world_spawn_sync` timed out at the 900s `cargo test --lib --no-run` compile step on Windows.)

## 3. Decoded Wire Capture Summary (`wire_capture.json`)

| Scenario | Server `minecraft:brand` (`S->C`) | Client `minecraft:brand` (`C->S`) | Client `horizon:mod_list` (`C->S`) | Client `horizon:payloads` (`C->S`) | Result |
| --- | --- | --- | --- | --- | --- |
| `direct_horizon` | `"horizon"` | `"vanilla"` then `"horizon"` | `[{"id": "probe_shared", "version": "0.1.0"}]` (20 B) | 32 B (`horizon:attachment_sync`) | `rc=0`, entered `play` |
| `horizon_behind_velocity` | `"horizon (Velocity)"` | `"vanilla"` then `"horizon"` | `[{"id": "probe_shared", "version": "0.1.0"}]` (20 B) | 32 B (`horizon:attachment_sync`) | `rc=0`, entered `play` |
| `vanilla_26_3` | `"vanilla"` | `"vanilla"` | **none sent** | **none sent** | `rc=0`, entered `play` |
