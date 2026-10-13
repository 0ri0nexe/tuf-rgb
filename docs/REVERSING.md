# Reversing the ASUS TUF keyboard lighting

Target: ASUS TUF Gaming F15 FX507VV (BIOS FX507VV.329), Linux 6.17.
Goal: understand how Armoury Crate drives the keyboard backlight, then reimplement it.

## TL;DR

* The keyboard is a plain PS/2 (i8042) keyboard. There is no USB/I2C HID lighting device (the only ASUS HID node, `ITE5570 0B05:19B6`, carries the airplane-mode key only).
* Lighting goes **Armoury Crate → WMI `DEVS` → ACPI `TU56`/`TU57` → EC mailbox**.
* The payload is the classic Aura packet without its report ID:

  | byte | meaning |
  |------|---------|
  | 0 | command: `0xB3` set (temporary), `0xB4` set and save |
  | 1 | mode: 0 static, 1 breathing, 2 color cycle, 3 strobe |
  | 2,3,4 | R, G, B |
  | 5 | speed: `0xE1`, `0xEB`, `0xF5` |

  WMI packs it as `arg0 = b0 | b1<<8 | b2<<16 | b3<<24`, `arg1 = b4 | b5<<8` on device `0x00100056`. Power states use device `0x00100057` with `arg0 = 0xBD | save<<10 | flags<<16` (flags: bit1 boot, bit3 awake, bit5 sleep, bit7 keyboard).
* This machine reports keyboard type `0x10004` = **TUF_ONE_ZONE_RGB**: one color for the whole keyboard. Everything beyond the 4 firmware modes (Aura Sync, rainbow, reactive...) is rendered on the host and streamed as temporary static colors.
* Linux `asus-wmi` already exposes exactly this as `kbd_rgb_mode` / `kbd_rgb_state`. What was missing is the host-side effect engine, profiles and UI: that is `tuf-rgb`.

## 1. Getting Armoury Crate

The per-model download (`ArmouryCrateAuraCreatorInstaller_ASUS_Z_V3.3.6.0_17877.exe`) is a web bootstrapper:

1. Outer exe: small MSVC stub whose `.rsrc` embeds an Inno Setup 6.1 installer at offset `0x25668` plus a 7z archive. The Inno loader table (`rDlPtS`) is valid for the embedded exe only, which is why innoextract rejects the outer file. Carving the inner PE works.
2. The 7z holds `ArmouryCrateInstaller.exe`, which references the offline bundle `https://dlcdnets.asus.com/pub/ASUS/mb/14Utilities/Armoury_Crate_Full_Installation_Package.zip` (5 GB, AC Full Package 1.4.4.3).

## 2. Decrypting the full package

`Armoury Crate Full Installation Package.exe`
(sha256 `56cf2783…155e9e`) ships `SupersetPackage_{Core,HAL,Others}.zip` and `AsusGamingCenterInstallPackage.zip`, each containing `<name>1.enc` + `<name>2.dat`.

Recovered in `FUN_140026300` ("DecodeZip") with REA/Ghidra:

* `FUN_140014650`: hex-decodes a DER blob at `0x1400d6dd0` (0x4A8 bytes), then `CryptDecodeObjectEx(PKCS_RSA_PRIVATE_KEY)` + `CryptImportKey`. The RSA-2048 private key is embedded in the binary.
* `FUN_140014410`: decrypts `1.enc` in 256-byte blocks with `CryptDecrypt` (PKCS#1 v1.5, CryptoAPI little-endian ciphertext, so each block is byte-reversed before standard RSA).
* `FUN_140018940`: strips a trailing 4-byte CRC32 (table-driven, init `0x56`, no final XOR) and checks it.
* The plaintext (100 KiB) is the head of a regular zip; `2.dat` is the unencrypted tail.

`re/tools/acdecrypt.py <exe> <x1.enc> <x2.dat> <out.zip>` reimplements this.
Only the first 100 KiB of each multi-GB zip is encrypted: it hides the local headers, nothing more.

## 3. Finding the lighting code

* `SupersetPackage_HAL/HAL/AacNBDTHal`: WiX Burn bundle → `AacSetup.msi` → `AacNBDTHal_x64.dll`
  (sha256 `cbd03a5c…006106`), the notebook/desktop lighting HAL.
* `AsusGamingCenterInstallPackage` → `ARMOURYCRATEPackage_5.9.14.0_x64.msix` →
  `AURA Service.zip` → Burn bundle → `AuraServiceSetup.msi` → `LightingService.exe`
  (sha256 `b473755b…d9f6`), the effect engine that loads the HALs.

### AacNBDTHal_x64.dll

* `FUN_18001a220` (WriteAuraDeviceInfoToRegistry): for the virtual TUF PID `0xFF30` it calls
  `ACPIWMI.dll!AsWMI_NB_GetDeviceStatus(0x0010005A)`, falling back to `0x00100055`:

  | DSTS value | keyboard |
  |-----------|----------|
  | `0x10000` | TUF_ONE_ZONE_RED |
  | `0x10001` | TUF_QWER_WASD |
  | `0x10002` | TUF_FOUR_ZONE |
  | `0x10003` | TUF_PERKEY |
  | `0x10004` | TUF_ONE_ZONE_RGB |

* `FUN_18000cf00` maps Armoury Crate effect IDs to firmware modes:
  `1→0 static, 2→1 breathing, 4→2 color cycle, 5→3 strobe (TUF), 8→0xB, 10→0xC, 0x11→0xA, 0xAD→0xD, 0xAE→0xE`.
* `FUN_180010b90` builds the packet `[0x5D, 0xB3, mode, R, G, B, speed, direction]`, speed index 1/2/3 → `0xF5/0xEB/0xE1`, sends it, then sends `0xB4` (apply) and `0xB5` (save).
* `FUN_18001cdc0` (SetEffect, direct/per-frame mode) has branches for ROG per-key and zone keyboards but **none for TUF types 0xA0..0xA4**: on TUF, host effects are pushed through
  the single-color path, one packet per frame.

### LightingService.exe

* `FUN_005e7dc0` reads `DSTS 0x00130021` through `ASUSATKWMI_WMNB`; low word 4 = "Tuf Notebook".
* `FUN_005bdbc0` is its generic `DEVS`/`DSTS` WMI wrapper; it is only used for detection here.

## 4. Firmware side (this machine)

`re/probe.sh` (root, read-only) results:

```
0x00100055 -> 0x10004   TUF_ONE_ZONE_RGB
0x00100056 -> 0x10000   RGB mode device present
0x00100057 -> 0x10000   RGB state device present
0x0010005A -> absent    (HAL falls back to 0x100055)
0x0010005B -> 0x0
0x00130021 -> 0x30004   product type 4 = TUF notebook
```

DSDT (`WMNB`):

```
If ((IIA0 == 0x00100056)) { Return (^^PC00.LPCB.TU56 (IIA0, IIA1, IIA2)) }
If ((IIA0 == 0x00100057)) { Return (^^PC00.LPCB.TU57 (IIA0, IIA1)) }
If ((IIA0 == 0x0010005B)) { Return (^^PC00.LPCB.TU5B (IIA0)) }

Method (TU56, 3) { MC58 = Arg0; MC5C = Arg1; MC60 = Arg2; MC7D = 0x03; MC7C = One }
Method (TU57, 2) { MC58 = Arg0; MC5C = Arg1;              MC7D = 0x05; MC7C = One }
Method (TU5B, 1) { MC58 = Arg0; MC7D = 0x04; WCMR(); ...read 8 bytes MC6C..MC73 }
```

The BIOS does no validation: it copies the WMI arguments into the EC mailbox (EC RAM `0x58..0x7D`), sets a command byte (3 = RGB mode, 5 = RGB state, 4 = read back) and rings `MC7C`. All interpretation happens in the ITE EC firmware.

## 5. Open questions

* Which "raw" modes (4..11) the EC actually renders: run `re/hwtest.sh`.
* Which speed byte is fast: kernel names 0xE1 "slow", Armoury Crate maps its own speed index 3 to 0xE1. `re/hwtest.sh` settles it.
* `DSTS 0x0010005B` returns an 8-byte buffer from the EC (`TU5B`), possibly the current lighting state. asus-wmi only returns the first dword (0 here).
* The direction byte of the Aura packet is not forwarded by WMI (only 6 bytes fit in two dwords), so firmware animations have no direction control on this model.

## Reproduce

```
re/tools/acdecrypt.py acfull.exe SupersetPackage_HAL1.enc SupersetPackage_HAL2.dat HAL.zip
re/tools/unpack.sh <dir>                 # recursive zip/msi/cab/msix unpack (7z)
re/tools/scanconst.py <dir>              # find PE files referencing TUF WMI device IDs
sudo bash re/probe.sh                    # firmware facts, ACPI tables
```
