# Raspberry Pi plugins

Open **Skills → Set up and use your Raspberry Pi**, or **Connections → MCP
servers → Raspberry Pi**. Choose your model. For a Pi computer, Sovra searches
the local network for eight seconds; choose your computer name from the results
or enter it yourself, add your account and password once, then press **Connect and chat**.
**Find my Pi** repeats the search. Other computers can appear too: discovery is
a suggestion, and connecting checks the reported Pi model. For Pico, connect a data USB
cable and install MicroPython for the exact board first. The saved Pi opens with
one click on later visits. Missing access opens chat with a concrete setup error;
a plugin being **Ready** does not claim the hardware answered.

The chat's **GPIO pins** card belongs to that selected Pi. **Build a project** fills
the coding shortcut; describe the project and send it. The in-app coding workflow
uses Sovra's selected model, writes the source, saves it in the library, then
automatically deploys it to this Pi. Other Pi plugins are excluded from this chat's
tools, and the runtime also checks the target on each call.

**Details** on the connection card contains lessons, health checks, tool switches,
pause, rename and removal. Guides and complete beginner examples work without
hardware or a chat model. **Save to library**, **Copy**, **Save code** and **Copy code**
keep them available for later.

## Building and deployment

Supported bundles are Python (`main.py`), Node (`main.js`) and static websites
(`index.html`) on Pi computers, and MicroPython (`main.py`) on Pico. Files must be
relative source paths, at most 64 files, 256 KB each and 2 MB overall. This does not
install dependencies, compile framework projects or provision an operating system.
Python/Node and any required libraries must already be installed on the Pi.

Computer deployment verifies the reported model, keeps source in a new directory
under `~/.local/share/sovra/projects`, starts it as the SSH account and reports the
one-second process check. Static websites use port 8080. Earlier source directories
are retained; a previous Sovra process is stopped only after its recorded process
identity, group and working folder match. Other processes are left alone. A started process is not a test of physical effects. Pico deployment
requires [mpremote](https://docs.micropython.org/en/latest/reference/mpremote.html)
on this computer (or an executable path supplied by `SOVRA_MPREMOTE_PROGRAM`). It
requires exactly one Raspberry Pi USB serial device, checks MicroPython's model and
USB identity, copies into a new folder and preserves the old `main.py` before
installing a launcher and requesting a reset. Connection/deployment pauses the
previous Pico program. Firmware flashing is not performed.

Source is saved before delivery; a failed save prevents deployment. A failed or
interrupted delivery keeps saved source and reports the problem. If cancellation
or timeout occurs after transfer starts, inspect the Pi before retrying: a remote
process or reset can already have happened. Generated code is never run in Sovra.

## Included skills and GPIO pins

Every installed profile includes seven reference/check tools under **Details**: **Setup steps**, **First project**, **GPIO pins**, **Blink a light**,
**Read a button**, **Fix a problem**, and its computer/USB check. Connection verification and the coding workflow's
project delivery are also bundled (nine MCP tools total). No extra skill
download is needed. The Skills page describes these capabilities and opens the
Pi setup. Normal per-tool switches and plugin pause still apply.

**GPIO pins** opens a native connector map. Select a pin to see its physical
number, code number and purpose. Power, ground, board controls and reserved
identification pins have explicit labels. This is a reference, **not live pin
readings**. It does not measure voltage or change pin direction or state.
Asking the connected helper about GPIO also returns this map directly in chat,
without needing to open the device card first. These reference cards cannot
change the Pi selected for the conversation.

- The 40-pin map covers newer classic Pis, Zero and keyboard models. Keyboard
  connectors are rotated; always locate the printed pin 1/40 markings.
- Original Model A uses 26-pin P1. Original Model B shows revision 1 and revision
  2 separately: pins 3/5/13 differ. P5 auxiliary pads are shown separately for
  revision 2; the map does not assume their pins are fitted.
- All Pico versions show their 40 edge positions with USB at the top. There are
  26 exposed GPIOs; power, analogue reference and control pins are distinct.
- Compute Modules expose signals through a carrier. Its exact model, revision,
  connectors and voltage configuration are needed before physical wiring or
  runnable GPIO code is suggested. No universal carrier map is invented.

LED and button lessons include parts, physical wiring, code numbering, expected
results and common faults. LED examples finish after ten flashes; button examples
finish after 30 seconds. Code is supplied for review and manual execution on the
board. It is never executed in Sovra. Disconnect power before wiring, use an LED
series resistor, and never connect 5 V or a motor directly to a GPIO signal.

Pin facts were verified against the official [40-pin layout](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#gpio),
[Pico board layouts](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html#board-layout),
and [GPIO Zero's maintained legacy connector data](https://github.com/gpiozero/gpiozero/blob/master/gpiozero/pins/data.py).
Examples use [GPIO Zero](https://gpiozero.readthedocs.io/en/stable/recipes.html)
or [MicroPython's RP2 pin API](https://docs.micropython.org/en/latest/rp2/quickref.html#pins-and-gpio).

Every profile is a real bundled MCP server, managed by the existing service
system. Pause, reconnect, rename, remove and individual tool switches work as
for other MCP servers. Adding the same profile with the same connection again
keeps its settings. Several physical computers can use the same model profile
with different saved computer names.

## Device checks

Nearby discovery uses local mDNS SSH/workstation advertisements and checks the
default `raspberrypi.local` address on port 22. It returns at most 32 computer
names, never signs in, and stops after eight seconds (plus up to one second for
cleanup). Some networks block discovery or do not advertise computer names;
manual entry remains available. The runtime owns and cancels the search.
Discovery uses [mdns-sd](https://docs.rs/mdns-sd/0.22.0/mdns_sd/); existing runtime
dependency boundaries are unchanged. Network names do not supply account names,
prove hardware identity or grant access.

Adding a plugin starts its tools; it does **not** establish a hardware connection.

- **Pi computers:** **Connection details** accepts the Pi's computer name/address,
  account and password. On the first sign-in, confirm the displayed connection
  identity. Sovra checks the actual reported model before saving access. Later
  connections reuse that identity and stop before sending a password if it changes.
  Passwords stay out of chat, MCP arguments, events and saved profiles. Release
  builds use the OS credential store; debug `cargo xtask dev` uses the existing
  owner-only development credential directory outside the checkout. Removing the
  connection also removes its saved sign-in; a storage failure is reported.
  Enable remote access on the Pi first and install Python 3. Password-based pairing
  uses the bundled [russh SSH client](https://docs.rs/russh/0.65.4/russh/), so no
  separate SSH client is needed. Leave the password blank to reuse saved access or
  an existing trusted OpenSSH key connection. **Check Pi health** reads the actual
  reported model, uptime, memory, root storage free space and temperature. Missing
  readings say **Not reported**. Setup help works before a device is available.
- **Pico:** **Check USB connection** enumerates serial devices using Raspberry Pi's
  USB vendor ID. It never opens a serial port or interrupts a project. Detection
  does not establish the exact model, installed firmware or program health. A
  BOOTSEL drive and firmware with another USB identity may not appear. Thonny
  and the official model-specific MicroPython download are used for the first
  project. This read-only USB check is separate from **Connect and chat**, which opens MicroPython to verify the model.

Computer checks use a remembered, explicitly confirmed host identity or strict
OpenSSH host-key verification. They never forward connections or run local command
hooks. The OpenSSH fallback ignores SSH configuration files and never prompts.
Only the configured host/account are used; a model cannot supply another
address or a command. A fixed read-only Python probe is sent through SSH's input,
never assembled from user text. Health checks do not change the device. Project
delivery is a separate operation invoked by the coding workflow.

Each SSH check has a 15-second deadline and separate 16 KiB output limits.
Pairing has a 150-second deadline, including up to 120 seconds for identity
confirmation; cancellation closes the SSH socket and stops the confirmation wait.
Child processes are killed on cancellation; the existing MCP program group owns them. USB enumeration runs off the runtime workers and never opens devices.
Project delivery has a bounded 30-minute MCP deadline, with shorter per-process deadlines. No credentials are added to profiles. Saved password access and key-based access
share the same fixed scripts and model checks. Only when no saved access exists
does SSH use the local user's available keys/agent and trusted host records.
A broken saved credential or changed identity never silently falls back.

## Model coverage and sources

Checked against Raspberry Pi's official documentation on **26 November 2026**.
There are **39 selectable profiles**. RAM sizes, board revisions, regional
keyboard layouts, radio options, storage capacities and desktop kits use their
matching profile; they do not require duplicated plugins. Bare RP2040/RP2350
chips, prototype boards, carrier boards and accessories are not standalone Pi
models. This catalogue is bundled and does not claim to auto-update for future
products.

| Family | Profiles | Official evidence |
| --- | --- | --- |
| Classic computers | 1 A, 1 B, 1 A+, 1 B+, 2 B, 3 A+, 3 B, 3 B+, 4 B, 5 | [Single-board computers](https://www.raspberrypi.com/documentation/computers/sbcs.html), [revision/model identification](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html) |
| Zero | Zero, Zero W, Zero WH, Zero 2 W, Zero 2 WH | [Zero models](https://www.raspberrypi.com/documentation/computers/zero.html), [Zero 2 WH reference](https://www.raspberrypi.com/news/what-can-you-build-with-raspberry-pi-zero/) |
| Keyboard computers | 400, 500, 500+ | [Keyboard computers](https://www.raspberrypi.com/documentation/computers/keyboard-computers.html) |
| Compute Modules | CM1; CM3 and Lite; CM3+ and Lite; CM4 and Lite; CM4S and Lite; CM5 and Lite; CM Zero and Lite | [Compute Modules](https://www.raspberrypi.com/documentation/computers/compute-module.html), [CM4S Lite](https://www.raspberrypi.com/news/new-memory-variants-for-the-raspberry-pi-compute-module-family/), [Compute Module Zero](https://www.raspberrypi.com/products/compute-module-zero/) |
| Pico | Pico, H, W, WH; Pico 2 and Pico 2 W, each with and without headers | [Pico boards](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html) |

Setup links use [Raspberry Pi Imager](https://www.raspberrypi.com/software/),
[MicroPython](https://www.raspberrypi.com/documentation/microcontrollers/micropython.html)
and the official [remote access guide](https://www.raspberrypi.com/documentation/computers/remote-access.html).

### Versions within a profile

The catalogue includes current products and older boards people still own.
Selecting a profile covers these versions without an extra technical choice:

| Profile | Included versions |
| --- | --- |
| Pi 1 A+ and Pi 1 B | 256MB and 512MB; all board revisions |
| Pi 2 B | All processor/board revisions; Imager chooses a compatible system |
| Pi 4 B | 1GB, 2GB, **3GB**, 4GB and 8GB |
| Pi 5 | 1GB, 2GB, 4GB, 8GB and 16GB |
| Pi Zero | Original and camera-connector revisions; W/WH and 2 W/2 WH have separate choices |
| 400, 500, 500+ | Every regional keyboard layout, standalone unit and desktop kit |
| Compute Module 4 and 4S | All 1/2/4/8GB memory and storage variants, plus CM4 temperature options; Lite is a separate choice |
| Compute Module 5 | All 2/4/8/16GB memory, storage and wireless variants; Lite is a separate choice |
| Compute Module 1, 3 and 3+ | All storage variants; available Lite versions have separate choices |
| Compute Module Zero | 8/16GB storage and wireless options; Lite is separate; development boards use the fitted module's profile |
| Pico and Pico 2 | All eight wireless/header combinations have their own choices |

Sources: [classic board variants](https://www.raspberrypi.com/documentation/computers/sbcs.html),
[3GB Pi 4 announcement](https://www.raspberrypi.com/news/a-new-3gb-raspberry-pi-4-for-83-75-and-more-memory-driven-price-increases/),
[Compute Module variants](https://www.raspberrypi.com/documentation/computers/compute-module.html),
[Compute Module Zero specifications](https://www.raspberrypi.com/products/compute-module-zero/).

## Implementation and verification

- `sovra-core::raspberry_pi`: model catalogue, input validation, setup/project
  content, health interpretation and the local result reducer.
- `McpCommand::AddRaspberryPi`: validates and registers a bundled program through
  the same runtime service actor. The app supplies its executable once in the
  existing `mcp_sample_program` composition-root field.
- `sovra --raspberry-pi-mcp <setup-json>`: a windowless standard MCP stdio server.
  Every profile shares one implementation. Seven reference/check tools take empty
  input schemas; `connect_device` verifies identity and `deploy_project` accepts
  validated source bundles. Delivery is hidden from model tool offers and called
  by the saved coding workflow. Imported programs retain normal approval.
- `raspberry_pi_view.rs` and `raspberry_pi_gpio_view.rs`: native theme/token controls,
  model search, collapsed details and the in-chat AG-UI GPIO component.
- Regression coverage includes input injection, family-specific tool availability,
  real SDK negotiation/tool calls for every profile, result isolation and native
  layout at minimum/wide windows and 200% interface scale.

Software tests and local USB enumeration cannot certify operation on every
physical board. Model-specific hardware checks require real devices and the
prerequisites above. The project does not claim fleet hardware certification.
