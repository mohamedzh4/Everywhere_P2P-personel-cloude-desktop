# Everywhere for Mac

Use this Mac from any browser, anywhere. Nothing to install on the other laptop.
Needs macOS 13 or newer on an Apple Silicon Mac (M1 or later).

## On this Mac

1. Unzip and drag **Everywhere.app** to **Applications**.
2. Open it. The first time, macOS says it cannot check the app (it is free and
   not paid-signed by Apple):
   - macOS 15 and newer: open **System Settings > Privacy & Security**, scroll
     down and click **Open Anyway** next to Everywhere, then confirm.
   - macOS 13–14: right-click Everywhere.app, choose **Open**, then **Open**.
3. macOS asks for two permissions. Allow both, **before you leave home**:
   - **Screen & System Audio Recording**, so the browser sees the screen and
     hears the sound. After switching it on, quit and reopen Everywhere.
   - **Accessibility**, so the browser can move the mouse and type.

   If you missed a prompt, the Everywhere window shows **Open Settings** next to
   anything still missing.
4. The window shows the web page to open, your **PC code** and your
   **password**.

Leave Everywhere running (the window can be minimised) while you are away.
Closing the window stops it. To keep the Mac awake, turn on **System Settings >
Displays > Advanced > Prevent automatic sleeping when the display is off**
(or Energy settings on older macOS).

## On the other laptop

Open the web page in Chrome or Edge, type the PC code and the password, and
click **Connect**. Keys follow the Mac's layout: the Windows key on a PC
keyboard acts as **Command**.

## Good to know

- **Stop sharing** in the window turns off access until you click **Start
  sharing**.
- After an update, macOS may ask for the permissions again.
- Settings and the log: `~/Library/Application Support/Everywhere/`.
- To remove Everywhere: delete the app, that folder, and its entries in
  Privacy & Security.
