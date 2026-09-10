# v6.5.45 upstream rebase

Based on the user-supplied current `ossm-main (1).zip` from `ossm-rs/ossm`.

Retained fork changes:
- ESP32-S3 owner webpage and HTTP Basic authentication
- persistent owner limits and machine limits
- Wi-Fi setup/settings storage
- owner E-stop and BPM controls
- XToys BLE integration and accumulated XToys fixes
- 168 KiB heap / Wi-Fi+BLE coexistence work
- 10 ms motion scheduling where applicable
- existing Ruckig recovery and streaming/hold fixes

Upstream position-feedback / PCNT step-dir infrastructure is preserved.

Parked experimental integrations are not included: SexCode/Sexync, PC web-host,
Web Bluetooth room, ToyControl, or Fleshlight Launch emulation.
