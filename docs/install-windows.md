# Everywhere for Windows

Use this PC from any browser, anywhere. Nothing to install on the other laptop.

## On this PC

1. Unzip and double-click **Everywhere.exe**.
2. If Windows asks about the firewall, click **Allow access**.
3. The window shows three things:
   - the web page to open (https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/)
   - your **PC code** (like `482-913-007`)
   - your **password** (like `k7mq-2xpn-r4dw`)

Leave the window open while you are away. Closing it stops Everywhere.

The status page `http://127.0.0.1:47800` (open it on this PC only) shows who is
connected, lets you disconnect them, and makes a new password.

## On the other laptop

1. Open the web page in Chrome or Edge.
2. Type the PC code and the password, click **Connect**.
3. Click the picture to control your PC. **Full screen** sends keys like Esc and
   Alt+Tab to your PC; hold Esc to get the mouse back.

## Good to know

- **Keep the password secret.** The PC code is only an address. After 5 wrong
  passwords your PC slows down answers; after 20 in a day it stops answering
  until you click **Unlock** on the status page.
- **Some networks block direct connections** (some universities, some hotels).
  Everywhere then says so. A phone hotspot usually works.
- **Sound** needs Windows' output at 48 000 Hz (Sound settings > your speakers >
  Properties > Advanced).
- **Programs running as administrator, the lock screen and UAC prompts** cannot
  be controlled yet.
- Settings are in `%APPDATA%\Everywhere\config.json`.
