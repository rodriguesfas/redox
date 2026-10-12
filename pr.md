# WiFi driver batch — progresso persistente

Atualizado: 2026-10-11 · branch `dev` (redox-os)

## Git

- `dev` = master + cadeia WiFi (20 commits). Ath9k esquadrinhado p/ histórico limpo.
- Push pendente: remoto é upstream público `github.com/redox-os/redox` — aguardando OK do usuário.

## Commits da sessão (mais novos primeiro)

1. `ac3ec7383` ath9k: upload real de firmware p/ SRAM (halt→stream→start→poll; 16/16 testes)
2. `34aaa6d35` iwlwifi: última cópia do conversor FC bugado migrada p/ net80211 (−141 linhas)
3. `e717b29e8` net80211: `Ccmp::decrypt`/`Gcmp::decrypt`, TKIP→Err, gcmp compila — zero `todo!()`
4. `2a26b9fbe` net80211: Minstrel table descartada + `Ccm<U8,U13>` (9/9)
5. `540aee10c` 4 wrappers *_net migrados p/ conversor central (−552 linhas duplicadas)
6. `40cd77ad8` net80211: conversor central 802.11↔Ethernet, FC ToDS correto 0x0108 (5 testes)
7. `3af47daee` ath9k_net: daemon network scheme
8. `0a091561f` ath9k: `Firmware::load` real+fallback
9. `3dff3e591` ath9k: lib TDD

## Bugs reais corrigidos

- FC STA→AP TX era FromDS (0x0208) — copiado em 4 wrappers + lib iwlwifi. Agora 0x0108 em tudo.
- `Minstrel::new` descartava RateTable populada → rate control sempre vazio.
- `Ccm<Aes128,U12,U8>` invertido panicava no 1º encrypt CCMP; ordem certa é `<Aes128,U8,U13>`.
- `Ccmp::decrypt` todo!() ⇒ panic se RX WPA2 chegasse. Implementado + teste MIC adulterado.

## Suite

- iwlwifi 27 · rtw88 26 · mt7921d 29 · ath9k 16 · net80211 10 (+gcmp) · wrappers OK
- `cargo check` nos 4 daemons: Finished sem erro.

## Fila

- [PENDING] Upload real de firmware nos outros 3 drivers (rtw88/mt7921d/iwlwifi precisam de staging via TXQ — chip-specific); molde: ath9k `Mac::upload_firmware`.
- [PENDING] Teste hardware/QEMU passthrough WiFi.
- [PENDING] Push origin dev + PRs (aguardando OK).

## Notas

- Progresso fica DENTRO do repo (arquivo de fora do repo estava sendo apagado por processo externo).
- Sem stash; TDD; commits por peça; merge→dev após implementar.
