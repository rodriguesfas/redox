#!/usr/bin/env bash
# hw-validate.sh — validacao dos drivers WiFi no hardware alvo (Acer Nitro ANV15).
#
# O que faz:
#   1. confirma que o chipset WiFi aparece (MediaTek MT7921, 14c3:7961)
#   2. confirma que IOMMU group esta isolado (vfio)
#   3. (opcional, com --dangerous) faz unbind do mt7921e no host e
#      passa o PCI device pro QEMU com imagem Redox que contem os drivers
#
# Uso:
#   ./hw-validate.sh                -> apenas verifica/sair (modo seguro)
#   ./hw-validate.sh --dangerous    -> executa unbind + passthrough (WiFi do host cai)
#
# Saindo de --dangerous: recarregar mt7921e manualmente.

set -euo pipefail

BDF=${BDF:-0000:3e:00.0}
VENDPROD=${VENDPROD:-14c3:7961}
IMG=${IMG:-/home/fasr/SSD_BACKUP/Workspace/aios/redox-os/build/x86_64/base/harddrive.img}
QEMU=${QEMU:-qemu-system-x86_64}

echo "== HW-VALIDATE(0): ambiente =="
command -v $QEMU >/dev/null || { echo "sem qemu"; exit 1; }
[ -d /sys/kernel/iommu_groups ] || { echo "sem IOMMU"; exit 1; }

echo "== HW-VALIDATE(1): chipset WiFi presente? =="
lspci -nn -s ${BDF##*:} 2>/dev/null || true
PCI=$(lspci -n | awk -v bdf="${BDF}" '$1 == bdf {print}' 2>/dev/null || true)
if [ -z "$PCI" ]; then
  echo "PCI ${BDF} nao achado"
  exit 1
fi
echo "OK: ${PCI}"

echo "== HW-VALIDATE(2): IOMMU group isolado? =="
GRP=$(readlink /sys/bus/pci/devices/$BDF/iommu_group | awk -F/ '{print $NF}')
echo "group $GRP"
ls /sys/bus/pci/devices/$BDF/iommu_group/devices/
DEVS=$(ls /sys/bus/pci/devices/$BDF/iommu_group/devices/ | wc -l)
if [ "$DEVS" != "1" ]; then
  echo "IOMMU group $GRP NAO esta isolado ($DEVS dispositivos) — passthrough inseguro"
  exit 1
fi
echo "OK: group isolado"

if [ "${1:-}" != "--dangerous" ]; then
  echo "== modo seguro: parando antes de tocar no host =="
  exit 0
fi

echo "== HW-VALIDATE(3): unbind ${BDF} do driver do host =="
DRV=$(readlink /sys/bus/pci/devices/$BDF/driver || true)
echo "driver atual: ${DRV:-none}"
if [ -n "${DRV:-}" ]; then
  echo "$BDF" | sudo tee /sys/bus/pci/devices/$BDF/driver/unbind
fi
modprobe vfio-pci
sudo modprobe vfio-pci_qinv || true
echo "$VENDPROD" | tr ' ' '\n' | sudo tee /sys/bus/pci/drivers/vfio-pci/new_id >/dev/null 2>&1 || true

echo "== HW-VALIDATE(4): boot imagem Redox com passthrough WiFi =="
[ -f "$IMG" ] || { echo "imagem nao achada: $IMG"; exit 1; }

echo "Rodando: $QEMU -enable-kvm -m 8G -device vfio-pci,host=${BDF} -drive file=$IMG,if=nvme [...]"
$QEMU -machine q35 -enable-kvm -m 8G \
  -device vfio-pci,host=${BDF} \
  -drive file=$IMG,if=nvme,media=disk,format=raw \
  -vga std -display none -serial stdio
