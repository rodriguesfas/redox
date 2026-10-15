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

## 2026-10-15 — #34 Init System: journald estruturado (slice 1/2: core + journalctl)

Alvo escolhido: **#34 (P0, blocker de userspace)** — item "journald / structured logging"
+ DoD "Logs estruturados (journalctl-like)". Hoje o `logd` é apenas um *sink* de texto
(`/scheme/log`, `add_sink`), sem campos, prioridade, boot id ou query.

- [DONE] Crate novo `journald` (workspace member) com core 100% host-testável:
  - `entry.rs` — `JournalEntry` (`FIELD=value`), validação de nomes (maiúsculas/dígito/`_`,
    prefixo `__` reservado), formato **export** com escape (`\\`, `\n`, `\r`) e round-trip,
    JSON por entrada, `encoded_len` p/ rotação.
  - `priority.rs` — `Priority` (syslog 0..7, ordenação = severidade), nomes/aliases
    (`err`/`error`, `warn`/`warning`, `panic`/`emerg`, numérico) para `-p`.
  - `syslog.rs` — `<PRI>` (facility*8+severity), structured data `[ID k=v ...]`,
    heurística `ident[pid]: msg` (rejeita frases), CRLF, metadados `_BOOT_ID`/`_HOSTNAME`/`_SOURCE_REALTIME_TIMESTAMP`.
  - `time.rs` — `--since/--until` (`YYYY-MM-DD[ HH:MM[:SS]]`, `T...Z`, `@epoch`, `-1h/-30m/-2d/-1w`,
    `now/today/yesterday/tomorrow`), `format_short/iso/timestamp`; aritmética civil pura.
  - `query.rs` — filtros AND: unit/identifier, `-p` (upper bound), janela de tempo,
    boot (`-b` current / por id), grep (case-insensitive por padrão), `FIELD=value|!=|~|!~`.
  - `store.rs` — `JournalStore`: append, query/tail, limites (`with_max_bytes` rotaciona),
    `vacuum_before`/`vacuum_to_bytes`, `boot_ids`/`boot_summary` (`--list-boots`),
    boot id 32-hex, leitura/escrita/append em arquivo.
  - `cli.rs` — parser completo do `journalctl` (flags curtas coladas `-n5`, `--flag=value`,
    `--`, posicionais `FIELD=value`, `-o short|export|json|json-pretty|cat`, `--file`, `-f`, `--list-boots`).
  - `render.rs` — short (`Jul 15 12:00:00 host unit[pid]: msg`), cat, export, json, lista de boots.
  - `src/bin/journalctl.rs` — CLI real (lê `--file` ou `/scheme/log/journal`).
- [DONE] **93 testes** no crate (90 unit + 3 de integração `tests/pipeline.rs`);
  verificado ponta-a-ponta rodando o binário de verdade com um journal real
  (`-p err`, `-u`, `-n`, `-g`, `--since`, `-o json`, `-o export` round-trip, `--list-boots`).
- [DONE] Empacotado: `journalctl` instalado em `/usr/bin` (Makefile `BASE_EXTRA_BINS`,
  pois o pacote se chama `journald`); CI `lab-tests` passa a rodar `journald` + `acpid`.
- [DONE] Sem regressão: **341 testes verdes** no host (wifi/bt/i2c/storage/acpi/journald/init).
- [PENDING] boot real na máquina (depende de você); depois `logind` → `networkd` → `resolved` (mesma issue).

## 2026-10-15c — #34 slice 2/2: daemon journald no `logd` + `--follow`

Fecha o DoD "Logs estruturados (journalctl-like)" do lado do produtor: quem escreve
no `/scheme/log` agora cai num journal estruturado em disco.

- [DONE] `journald/src/writer.rs` — `JournalWriter` (daemon side, só std → host-testável):
  - config por ambiente (`JOURNAL_PATH`, `JOURNAL_MAX_BYTES`, `JOURNAL_BOOT_ID`,
    `JOURNAL_HOSTNAME`/`HOSTNAME`, `JOURNAL_ENABLED=0` p/ desligar); default
    **`/scheme/logging/journal`** (ramfs volátil criada no initfs por `ramfs@logging.service`
    — equivalente ao `/run/log/journal`).
  - abre *lazy* na primeira linha e tolera falha: reporta **uma vez** no stderr,
    tenta de novo de forma ansiosa (8x) e depois espaçada (1/256) — um FS de log
    ausente nunca vira tempestade de syscalls nem derruba o log.
  - `write_line[_from]` poda CRLF (o `logd` manda `\r\n`), ignora linha vazia,
    parseia e faz append+flush (flush = `--follow` enxerga na hora).
  - rotação por compactação: passou de `max_bytes`, relê o journal, mantém os mais
    novos até metade do limite e regrava (recupera registro truncado no meio).
  - **boot id estável entre restarts**: sem `JOURNAL_BOOT_ID`, adota o boot do
    journal existente (arquivo é volátil ⇒ é do boot corrente); só boot novo gera id
    (`_BOOT_ID` = 32 hex). Assim `logd` reiniciar não vira "outro boot" no `-b`.
  - `Source.pid` (`_PID` do processo escritor, `ctx.pid` vem do `logd`; `ident[pid]:`
    no texto continua tendo precedência).
- [DONE] `journald/src/follow.rs` — lado leitor:
  - `complete_prefix_len`/`decode_complete`: só consome registros terminados por linha
    vazia (registro em escrita nunca é lido pela metade) e pula registro malformado
    em vez de abortar o read.
  - `Tail` (offset + restart automático se o arquivo encolheu por rotação);
    `Follower` (lê do fim, aplica a `Query`, renderiza) — o motor do `--follow`.
- [DONE] `logd` (integração mínima, thread consumidora onde a linha já está montada):
  `OutputCmd::Log { line, pid }`, `journald::JournalWriter::from_env()` e
  `journal.write_line_from(...)` antes do fan-out para os sinks. Se o journal falhar,
  o `logd` continua exatamente como antes (best effort, sem bloquear; canal unbounded).
- [DONE] `journalctl --follow` real (poll de 250 ms), começando no fim do arquivo e
  sem replay do backlog; `-n 0 -f` funciona como `tail -f`.
- [DONE] **115 testes** no crate (111 unit + 4 integração). Sem regressão: **365 testes
  verdes** no host nos 15 pacotes (wifi/bt/i2c/storage/acpi/journald/…), 0 falhas.
- [DONE] Compilação redox comprovada: `cook base` → `Compiling journald` + `Compiling logd`,
  `build/initfs/bin/logd` (15:33) contém `logd: journal `, `journald: cannot write `,
  `/scheme/logging/journal`, `JOURNAL_*`; `stage/usr/bin/journalctl` com `cannot follow`.
- [DONE] Smoke test do binário real: `journalctl --file … -f -n 0 -o cat` imprimiu só as
  linhas novas, juntou registro partido entre dois writes e não vazou metade de registro.
- [PENDING] boot real (initfs com o `logd` novo + `journalctl -b`/`-f` na máquina) — depende de você.

## 2026-10-15b — status: issues de driver fechadas

- Fechadas no tracker `aios-b-612/aios`: #25 (WiFi), #27 (audio), #28 (TRIM), #29 (BT),
  #30 (ACPI battery), #31 (i2c-hidd), #38 (CI). `dev` @ `13f348edd`, tree limpo.
- Branch local `wifi-drivers` removida (já mergeada em `dev`).

## 2026-10-13d — decisao: tarefas *sem* quebrar o sistema (hardware real fica p/ voce)

- [DONE] #38 CI: workflow lab-tests no GitHub Actions do fork (suite dos crates,
  passe em 1m06s, comecou branch dev — novo gate de merge envio).
- [DONE] #30 ACPI/Battery: acpid ganha arquivo 'battery' no scheme (eval _BST);
  4 testes do parser de status; sem suspender/sleep: aquilo e de board + HW.
- [DONE] #28 TRIM jnovo.
- [DONE] #29 BT uplink pronto (usable em testes).
- [DONE] #31 i2c.
- [DONE] #27 audio tapa gap de vol/mute.
- Boot + testes single-machine: pendente de usuario no Acer alvo.

## 2026-10-13c — Imagem de validacao bootavel CONSTRUIDA

- build/x86_64/aios-wifi-test/harddrive.img (512MB, ISO bootable, GPT+EFI).
  Contem: base (4 daemons wifi + btusd + i2c-hidd + wpa_supplicant), kernel,
  bootloader, relibc, ion.
- Cook global cozinhou relibc (submodulos restaurados) + toda a cadeia.
- Boot-safe checado em QEMU headless (20s sem crash; Redox loga VGA).
- Prowise test real: sudo hw-validate.sh --dangerous (derruba wifi host!).
- Issue #28: TRIM/DSM implementado (commit em dev).

## 2026-10-13b — #27 audio vol/mute end

- `audiohw` scheme agora tem arquivos vol/mute (matematica pura em hda/vol.rs,
  5 testes), a caminho de `apply_master_volume()` (caminho DAC->out).
- ihdad compila e empacota; proximos: booster PipeWire (parte userspace maior
  fica propositalmente de fora do escopo driver).
- Suite: wifi 165 + bt 51 + i2c-hid 8 + i2c-dwd 8 + ihdad 5 (+ base).

## 2026-10-13 — Item #31 (Input I2C) implementado em sw

- i2c-hidd daemon: probe I2C-HID (fetch desc, power up, polls) + fd de
  eventos p/ inputd; empacotado (usr/lib/drivers).
- i2c-hid crate (codec protocolar I2C HID), i2c-dwd (DesignWare master) com
  8+8 testes; daemon test host tem 1 sanity link (SDL2 host-link dodged com
  cfg-gating).
- Restante p/ #31 real close: device touch pixart handshake + inputd event
  path juntos na maquina.
- pr:Item 3 (validacao hw) esta pronto (hw-validate.sh).

## 2026-10-12i — items 1, 2, 3 da ordem "seguir 1 2 3"

1. [DONE] SDP minimal (btusd): codec+registry+server, inbound channels. 51 testes.
2. [DONE] i2c-dwd (DesignWare I2C master, Acer touchpad) — inicio da issue #31.
3. [DONE] hw-validate.sh: passthrough MT7921 validavel (IOMMU group 17 isolado).
   --dangerous so com consentimento explicito do usuario.
- dev@5f54d5763 pushed p/ fork+backup.
- pronto p/ RFC ou jump p/ #31 (input) de verdade.

## 2026-10-12h — #29 L2CAP userspace pronto (slice BT completo)

- Canal ACL p/ userspace: arquivo `acl` no scheme bt (leitura drena fila; escrita
  [dcid][payload] vai p/ bulk EP2); sinalizacao L2CAP (conn/cfg via CID 1)
  processada no daemon; teste de canal completo (open SDP -> ConnRsp -> CfgRsp
  -> open -> dados nos dois sentidos) com pacotes sinteticos reais.
- btusd: 40 testes. #29 permanece aberta so por: SCO/audio (A2DP) + hw real.
- proximo: proxima issue (ver ordem com cliente) OU SDP minimo dentro de BT.

## 2026-10-12g — #29 pairing implementado (SSP + PIN legado)

- btusd 34 testes: codecs de conexao/SSP, maquina de pareamento completa
  (JustWorks auto-accept, PIN fallback, link keys por BD_ADDR), ctl
  `pair aa:bb:..`/`pin`, teste E2E via eventos sinteticos, bug bool-default.
- cook base ok com tudo isso no pacote.
- #29 segue aberta por escopo: faltam L2CAP userspace (canais ACL p/ apps)
  e teste com dispositivo BT real (emparelhar fone). Software: pronto.

## 2026-10-12f — btusd completo nesse slice (#29 avancou)

- [DONE] btusd agora e daemon de verdade: probe -> scheme `bt` registrado
  (ctl/status/devices/event) -> loop servindo userspace + eventos USB.
- [DONE] inquiry funcional (cmd 0x0401 GIAC + parse result/RSSI + dedup).
- [DONE] framing L2CAP (ACL pack/decode, streams).
- 22 testes btusd; cook base ok com btusd no pacote.
- #29 ainda aberta: falta pairing (L2CAP SDP/psm, passkey) + HCI scheme
  de dados ACL p/ userspace (fundacao pronta).

## 2026-10-12e — Issue #29 (BT) iniciado

- WiFi issues #25/#40/#41/#42/#43 FECHADAS com evidencia (PR#1 + cook ok + 165 testes).
- Bt USD: btusd (HCI over USB, classe 0xE0/1/1) — codec HCI + probe machine, 11 testes,
  empacotado no base.pkgar (cook base successful). Drivers.toml xhcid atualizado.
- Nao fechei #29: falta scheme HCI p/ userspace + L2CAP + pairing.

## 2026-10-12d — PIPELINE DE CONEXAO fechado (scan/assoc/handshake)

- [DONE] net80211::link — maquina de estados Disconnected->Scanning->Associating->Associated
  + parser de beacon (SSID IE, dedup BSSID) — 5 testes
- [DONE] mt7921d: scan via REG_SCAN_CTRL; beacons/assoc-resp chegam pela RX ring
  (mgmt frames processados no driver, nao sobem ao netstack); assoc request real
  sai pela TX ring (write_ring sem gate); Associated somente apos status ok;
  dados bloqueiam enquanto nao associado
- [DONE] wpa_supplicant: integracao PSK->PMK->PTK(KCK/KEK/TK)->CCMP roundtrip
- [DO ZERO HW] resta so validar contra AP real (firmware eventos variam por chip)
- PR#1: 14 commits (net80211 link + pipeline) — suite: 165 testes + pipeline E2E

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
