# ⚡ Logical Lunge: Native UI

This folder serves as a showcase for the **Native UI** edition of Logical Lunge.

> **Note:** The actual source code for this edition is actively developed on the [`native-ui` branch](https://github.com/KaanAlper/logical-lunge/tree/native-ui).

## Architecture
The Native UI edition is designed for absolute maximum performance and zero input lag.
- **Top Bar:** Drawn entirely with **Direct2D** and **DirectComposition**. It completely bypasses WebView2, meaning animations run directly in the Windows compositor.
- **Resource Usage:** Extremely lightweight. Perfect for laptops or gaming setups where every megabyte of RAM matters.
- **Future Roadmap:** The Super menu, right sidebar, and settings are currently being rewritten natively one by one to eventually remove all web dependencies.

👉 **[View the full Native UI Source Code & Development](https://github.com/KaanAlper/logical-lunge/tree/native-ui)**
