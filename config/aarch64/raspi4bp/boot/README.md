# Boot do Raspberry Pi 4B (board `raspi4bp`)

Arquivos que ficam na **raiz da partição EFI (FAT)** do cartão SD, ao lado do
`EFI/BOOT/BOOTAA64.EFI` (bootloader Redox, instalado pela imagem).

| Arquivo | O que é | Origem |
|---|---|---|
| `kernel8.img` | u-boot (alvo `rpi_4`) — é o "kernel" que o firmware do Pi carrega | pacote Ubuntu `u-boot-rpi` `2025.10-0ubuntu0.24.04.4`, arquivo `usr/lib/u-boot/rpi_4/u-boot.bin` (md5 `9f256cd7b7b109369a9c9230e8d0447b`) |
| `start4.elf` | firmware do VideoCore (Pi 4) | `raspberrypi/firmware` (master, baixado em 2026-10-16) |
| `fixup4.dat` | fixups do firmware (Pi 4) | `raspberrypi/firmware` (idem) |
| `bcm2711-rpi-4-b.dtb` | Device Tree do Pi 4B | `raspberrypi/firmware` (idem) |
| `overlays/disable-bt.dtbo` | overlay que libera a PL011 nos pinos GPIO 14/15 | `raspberrypi/firmware` (idem) |
| `config.txt` | configuração do firmware (UART, kernel, DT) | este repo |

O `kernel8.img` é **binário do u-boot (GPL)**, não o kernel do Redox: o kernel
Redox é carregado pelo bootloader EFI a partir da partição RedoxFS.

Este é o **mesmo u-boot** validado no QEMU `raspi4b` (probes 6 e 7b, 2026-10-16):
`docs/evidence/pi4/probe6-boot-completo.log` e `probe7b-login-shell.log`.

## Como esses arquivos chegam ao cartão

`scripts/make-sd-pi4.sh` (na raiz do workspace) copia a imagem
`redox-os/build/aarch64/pi4/harddrive.img` para `aios-edge-pi4-sd.img` e injeta
esta pasta na partição EFI via mtools (sem root). Depois é só `dd` no cartão.

## Reproduzir a origem

```sh
# u-boot (Ubuntu):
apt download u-boot-rpi:arm64      # ou https://packages.ubuntu.com/noble/u-boot-rpi
dpkg-deb -x u-boot-rpi_*.deb x/ && cp x/usr/lib/u-boot/rpi_4/u-boot.bin kernel8.img

# firmware (raspberrypi/firmware, branch master):
curl -fLO https://raw.githubusercontent.com/raspberrypi/firmware/master/boot/start4.elf
curl -fLO https://raw.githubusercontent.com/raspberrypi/firmware/master/boot/fixup4.dat
curl -fLO https://raw.githubusercontent.com/raspberrypi/firmware/master/boot/bcm2711-rpi-4-b.dtb
curl -fLo disable-bt.dtbo https://raw.githubusercontent.com/raspberrypi/firmware/master/boot/overlays/disable-bt.dtbo
```
