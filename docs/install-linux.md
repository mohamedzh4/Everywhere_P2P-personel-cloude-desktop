# Everywhere for Linux

Use this computer from any browser, anywhere. Nothing to install on the other
laptop. Needs a 64-bit (x86-64) Linux desktop running **X11 (Xorg)**.

## Wayland or X11?

Most new desktops (Ubuntu 22.04+, Fedora) start in **Wayland**, which Everywhere
does not support yet. Check with:

```bash
echo $XDG_SESSION_TYPE
```

If it says `wayland`: log out, click the gear on the login screen, choose
**Ubuntu on Xorg** (or "GNOME on Xorg", "Plasma (X11)"), and log in again.

## On this computer

1. Unpack and start it:

   ```bash
   tar xzf Everywhere-*-linux-x64.tar.gz
   cd Everywhere-*/
   ./everywhere
   ```

2. The Everywhere page opens in your browser (only on this computer) with the
   web page to open elsewhere, your **PC code** and your **password**.
3. For sound, install the PulseAudio tools if they are missing
   (`sudo apt install pulseaudio-utils`; they work with PipeWire too).

Leave it running while you are away; stop it with Ctrl+C in the terminal.

## Good to know

- **Stop sharing** on the page turns off access until you click **Start
  sharing**.
- The firewall must allow UDP for the app (most desktops allow outgoing
  connections and their replies by default).
- Settings and the log: `~/.config/everywhere/`.
- To remove Everywhere: delete the folder and `~/.config/everywhere/`.
