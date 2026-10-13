# RemotePlayForever

Steam Remote Play Together for any game: Steam games that never enabled it, non-Steam games, emulators, GOG and Epic titles.

Pick a game, press launch, send the link. Your friend joins from Steam or the free Steam Link app and needs neither the game nor a fast computer.

- No donor game, nothing to configure in Steam.
- Nothing is changed in the game's folder.
- Windows x64. English and Polish interface.

## Quick start

1. Start **Steam** and sign in.
2. Run `remoteplayforever.exe`.
3. On the **Launch** tab pick the game's `.exe` (or pick a title on the **Steam games** tab) and press **Launch with Remote Play Together**.
4. A few seconds later the **guest link** is on your clipboard. Send it to a friend.
5. Close the game or press **End session** to disconnect the guests.

The session card shows the link, an **Another guest** button (one link is for one person) and live switches for the guests' keyboard, mouse and controller.

Language, default guest permissions and clipboard behaviour are on the **Settings** tab.

## How it works

The app attaches to the running, signed-in Steam client through `steamclient64.dll` and calls its internal `IClientRemoteClientManager`, the same interface the Steam overlay uses for "Invite to Remote Play Together":

1. `BCreateRemotePlayGroup(AppID)` creates a Remote Play Together group for the game.
2. `BCreateRemotePlayInviteAndSession` creates a guest slot and returns the **invite link**.
3. `SetPerUser{Keyboard,Mouse,Controller}InputEnabled` grants input to guests.
4. `DisbandRemotePlayTogetherGroup` disconnects the guests when the game exits.

Two launch modes, chosen automatically:

| Game | How it starts | Session AppID | Guests see |
|---|---|---|---|
| Installed Steam game | Steam launches it under **its own AppID** (license, DRM and overlay behave as usual) | The game's AppID | The game window |
| Non-Steam game | From its `.exe`, with the **Steam overlay** (Shift+Tab, Steam Input, guest controllers) | `480`, used only as a label; nothing is installed | **Your whole desktop** |

> With a non-Steam game guests see your whole desktop. Close private windows before sending the link.

### The overlay in a non-Steam game

Steam only puts its overlay into games it launches itself, but the overlay library follows child processes on its own. A small windowless launcher (`remoteplayforever.exe --overlay-launch`, [src/overlay.rs](src/overlay.rs)) loads Steam's own `GameOverlayRenderer64.dll`, sets `SteamGameId` and `SteamOverlayGameId` to `480` and only then starts the game. Steam registers the game's process and window under that id, which is what makes Steam Input and guest controllers work.

### Surviving Steam updates

The interface is undocumented and the order of its methods shifts between client builds, so the app **never calls by index**. Every client method is an IPC stub that references its own name as a string; at startup the app builds a `name -> index` map from them (`scan_vtable_names` in [src/rpt.rs](src/rpt.rs)). If a method it needs is gone, the session reports an error instead of guessing.

The session runs in a separate helper process (`remoteplayforever.exe --host-session`), so a bad call into Steam's internals cannot take the window down.

## Guests

- They open the link in Steam or in the **Steam Link** app (Windows, macOS, Linux, Android, iOS, Android TV).
- They get the keyboard and mouse, and a controller when the game supports one (setting **Guest controller**: automatic / on / off).
- The stream's frame limit is set by the guest: Steam -> Settings -> Remote Play -> advanced client options -> **Framerate limit** (up to 240 FPS). The host cannot raise it.

### Controller ignored by a non-Steam game?

A non-Steam game is registered under id `480` (Spacewar), and Steam Input has its own layouts for that id: Spacewar's action set for Xbox controllers, an empty one for DualSense. Neither behaves like a plain gamepad, so the game sees no input. Once:

1. In the game press **Shift+Tab** and open **Controller settings**.
2. For **every** controller tab (yours and each guest's) choose a layout from the **Gamepad** template.

Steam remembers the choice per controller type. Guest controllers appear in that list only after the guest has joined.

## Command line

```bash
# Open the window and launch a game with a session right away
remoteplayforever.exe --run "C:\Games\MyGame\game.exe"

# Same in the console, no window (blocks until the game closes, prints the link)
remoteplayforever.exe --cli --run "C:\Games\MyGame\game.exe"

# Extra arguments for the game
remoteplayforever.exe --run "C:\Emulators\yuzu.exe" --args "-f" "-g" "C:\Roms\game.nsp"

# Console menu
remoteplayforever.exe --cli

# List installed Steam games
remoteplayforever.exe --list-games
```

## Limitations

- Windows x64 with a running, signed-in Steam client.
- A non-Steam game is shared as the **whole desktop**, not a single window.
- For a Steam game the app starts the game's default launch option. Picking another `.exe` from its folder (a mod loader, say) still starts the game through Steam.
- It relies on Steam internals. A large client update may need a new release. Each session is logged to `%LOCALAPPDATA%\RemotePlayForever\launch.log`.
- Not affiliated with Valve. Use at your own risk.

### For maintainers: when Valve changes a signature

Method names resolve on their own, but **arguments** can change (`BCreateRemotePlayGroup` and `GetLocalRemotePlayTogetherGroupID` started taking the AppID at some point). Every client method is an IPC stub that serializes its arguments in order, so a disassembly shows the size of each argument and of the return value. The real call order is also visible in Steam's own interface code, `Steam\steamui\*.js` (`SteamClient.RemotePlay.*`).

## Building

Rust 1.85+ (edition 2024):

```bash
git clone https://github.com/KayZedd/remoteplayforever.git
cd remoteplayforever
cargo test
cargo build --release
```

The binary is `target\release\remoteplayforever.exe`. For an installer (Inno Setup 6): `./build_installer.ps1`.

Layout:

| File | Role |
|---|---|
| [src/rpt.rs](src/rpt.rs) | Session host: Steam client attachment, group, invites, guest input |
| [src/overlay.rs](src/overlay.rs) | Launcher that starts a non-Steam game with the Steam overlay |
| [src/session.rs](src/session.rs) | Line protocol between the app and the session host |
| [src/runner.rs](src/runner.rs) | Launch plan and the game + session lifecycle |
| [src/gui.rs](src/gui.rs), [src/ui.rs](src/ui.rs) | Window and console interfaces |
| [src/i18n.rs](src/i18n.rs) | Interface language |

## Credits

- [RemotePlayWhatever](https://github.com/m4dEngi/RemotePlayWhatever) by m4dEngi: the original technique.
- [RemotePlayForge](https://github.com/okabeeer/RemotePlayForge) by okabeeer (MIT): resolving methods by name and the group -> invite -> input sequence.

## License

[MIT](LICENSE). "Steam" and "Remote Play Together" are trademarks of Valve Corporation.
