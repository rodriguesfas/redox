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

## 2026-10-12c — rings reais em TODOS os drivers

- [DONE] Molde ath9k replicado em rtw88 (33/33), mt7921d (32+35), iwlwifi (33/33).
- Contrato uniforme: descritores {off,len} + head/tail MMIO, slots 2048B,
  gates EWOULDBLOCK/EINVAL/ENOBUFS, handle_tx_done consome, handle_rx_done noop.
- Suite total: 165 testes verde nos 5 crates.
- `cook base - successful` com os 4 daemons ring-ificados no base.pkgar.
- PR#1 (fork): 12 commits, todo o trabalho; dev backup em fork+local bare.

## 2026-10-12b — datapath ath9k real

- [DONE] Rings RX/TX reais no ath9k: descritores {off,len} + head/tail em MMIO,
  slots de 2048B na SRAM window; RX pop/limpa/recicla, TX publish+bump com
  backpressure (EWOULDBLOCK) — 5 testes com BAR fake (22/22 ath9k).
- Commit `597ce7a4` em dev; cherry-pick `4daf5a2` no branch do PR#1 (9 commits).
  Nota: hashes do PR diferem visualmente pq o gh jq cortou o fim do oid — sem divergencia real.

## 2026-10-12 (1,2,4 resolvidos; 3 em andamento)

- [DONE] 1 PR: https://github.com/rodriguesfas/redox/pull/1 (GraphQL bloqueado; REST OK). Upstream redox-os/redox requer GitLab MR — pendente de decisão.
- [DONE] 2 Wiper `.git`: evidência = árvore veio de backup que exclui `.git` em TODOS os recipes (restore 22:42). Nada ativo. Mitigação: push fork + bare local em SSD_BACKUP/git-backups/redox.git.
- [DONE] 4 Itens soltos do usuário commitados em dev (base pin+patch, relibc patch, intel-ucode, linux-firmware, aios-src, recipes/aios) e enviados ao fork+backup.
- [DONE] 3 r.base: `cook base - successful`, base.pkgar 49MB com 4 *_net em usr/lib/drivers + wpa_supplicant em usr/bin + pcid.d/*.toml. Fixes no caminho: .git mínimos+remotes+fetch revs (21 sources), member net/wpa_supplicant, import crypto::kdf, tipo &str do password.

- [PENDING] Build cookbook completo (`make` da base) para provar empacotamento
- [PENDING] Teste hardware/QEMU passthrough WiFi
- [TODO-USER] Decidir destino de `aios-src/`, `recipes/aios/`, patches soltos
