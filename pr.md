# WiFi driver batch — progresso persistente

Atualizado: 2026-10-11 · branch `dev` (redox-os) — **reconstruída após perda do .git**

## Incidente (resolvido em 22:5x) (IMPORTANTE)

- Às ~22:42 um processo externo (suspeita: sync/limpeza de `SSD_BACKUP`) **apagou os `.git`** de
  `redox-os/` e do workspace `aios/`.
- Arquivos-fonte intactos; histórico anterior se perdeu.
- Reconstruído: clone upstream transplantado, `dev` reposta sobre `e2bf6961e` (upstream master),
  commits refeitos por área a partir do estado final dos arquivos.
- `pr.md` agora vive DENTRO do repo e commitado (antes era apagado na raiz).

## Commits em dev (8, sobre e2bf6961e)

1. `85040ad48` iwlwifi driver (TDD)
2. `…` rtw88 driver (TDD)
3. `…` mt7921d driver (TDD)
4. `…` net80211 stack comum
5. `…` ath9k driver (TDD)
6. `…` 4 daemons `*_net` + workspace member ath9k_net
7. `…` wpa_supplicant + `platform/scripts/test-wifi.sh`
8. `…` .gitignore unignore + este arquivo

## Entregas (reconstruídas = estado final)

- Drivers: iwlwifi 29t · rtw88 29t · mt7921d 28+31t · ath9k 16t · net80211 10t (+gcmp)
- 4 daemons `*_net` compilam zero-erro
- Upload de firmware real (SRAM window + START/poll) nas 4 libs, testado com BAR fake
- Conversor 802.11↔Ethernet único em `net80211::ieee80211` (FC ToDS 0x0108 — bug FromDS eliminado)
- CCMP decrypt real (MIC verificado); zero `todo!()` no net80211
- Itens pré-existentes do usuário NÃO commitados: recipe.toml pins, aios-src/, recipes/aios/,
  patches nvmed/relibc, intel-ucode/, linux-firmware/

## Fila

- [PENDING] Build cookbook completo (`make` da base) para provar empacotamento
- [PENDING] Teste hardware/QEMU passthrough WiFi
- [TODO-USER] Decidir destino de `aios-src/`, `recipes/aios/`, patches soltos
