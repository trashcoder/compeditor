# CompEditor

CompEditor is a local Docker Compose editor for Linux, macOS, and Windows. The project sidebar lists the services, networks, and volumes in your Compose file. You can search each list, add entries, and resize the sidebar by dragging its right edge. The editor has tabs for configuration, connections, additional Compose settings, and Docker Hub information when available. You can switch between English and German in **File → Settings**. The app remembers your language preference.

## Getting started

Make the Linux AppImage executable, then run it. On systems without FUSE, start it with `--appimage-extract-and-run`. The Linux file picker uses the desktop portal and falls back to Zenity.

```sh
chmod +x CompEditor-0.1.0-x86_64.AppImage
./CompEditor-0.1.0-x86_64.AppImage
```

Try `examples/compose.yaml`. You can open it in the app or pass its path as a command-line argument. **Apply** puts form changes into the open document. In the **File** menu, **Save** writes the file and **Save as…** lets you choose another location. The **Edit** menu provides standard editing commands for the active text field. If a save conflict occurs, CompEditor keeps your local edits in memory. **Close file** returns to the start screen and prompts you about unsaved changes.

Choose **New project** and select an empty directory. CompEditor checks for existing files before creating `docker-compose.yml`, an empty `.env`, and the initial history.

Choose **Add service** in the project sidebar to create a service. Enter a name and optionally an image. The editor opens the new service immediately and records the addition in the history.

In the service editor, the restart policy dropdown offers `no`, `always`, `on-failure`, and `unless-stopped`, with common retry limits for `on-failure`. Leave the field unset to use the Compose default.

The **Image info** tab can extract environment parameters from tables in a Docker Hub image description. Choose **Configure for service** to open a parameter in the environment form. The app shows **Required** or **Optional** when the documentation specifies it. Otherwise, it shows **Not specified**. Docker Hub descriptions are free-form, so some images lack a parameter table.

The sidebar shows services declared in the open Compose file. This version leaves external `include` and `extends` files unloaded. Shared or ambiguous YAML values are read-only. CompEditor respects the structure of the YAML file. For full Compose validation, run `docker compose config --quiet`.

## Variables from `.env`

CompEditor loads the `.env` file next to the open Compose file. In wide windows, variables and their values appear to the right of the service editor by default. Drag the panel's left edge to resize it. Long variable names wrap. Yellow highlights variables with no direct or indirect reference in the open Compose file.

The **.env** button shows or hides the panel. In smaller windows, it temporarily opens the variables in place of the editor. Click a name or value to edit the value in its cell. References in the service editor link directly to the matching row. Missing or unresolved variables appear in red. Enable **Show preview values** to see the unresolved notice.

**Save to .env** writes edited values literally and refreshes the preview. If the file is missing, this action creates it. The preview uses values from `.env` and defaults in Compose expressions. Saving the YAML file preserves the placeholders.

For Docker Hub images, the service editor shows the description, pull count, stars, and last update date. Image names resolved from `.env` placeholders are supported. Metadata lookup requires an internet connection and covers Docker Hub images.

For public Docker Hub repositories, the image form offers a list of tags sorted by update time. Selecting a tag updates the tag portion of the image name and preserves the other Compose entries.

## History

**History** opens the change list with the newest entry first. YAML edits appear initially as drafts and become history entries when you save the Compose file. Edits to `.env` are recorded when you save the value.

**Restore** immediately restores the YAML file and `.env` to the selected saved revision, undoing the intervening changes together. CompEditor keeps the current service open and jumps to the affected variable when it can. Later revisions remain available until you make another change or close the file. When you reopen the file, the history ends at the last selected revision. Selecting a draft updates the document in memory. The files on disk change when you save.

CompEditor stores history beside each Compose file. For `compose.yaml`, the history file is `.compose.yaml.compeditor-history.json`. It contains earlier YAML and `.env` contents, including their values.

## Build and test

```sh
cargo run --locked -- examples/compose.yaml
cargo test --locked
cargo build --release --locked
```

On Linux, `scripts/package-appimage.sh` builds the AppImage in `dist/`. The GitHub Actions workflow builds it on Ubuntu 22.04 for broad Linux compatibility. On macOS and Windows, `cargo-packager` creates the native packages.

A push to `main` runs **Build CompEditor** for Linux x86-64, macOS on Apple Silicon and Intel, and Windows x86-64. After all four targets pass their builds and tests, the workflow publishes a GitHub prerelease tagged `dev-<commit-sha>`. Each prerelease contains one AppImage, two DMGs, and one Windows installer. The packages are unsigned.
