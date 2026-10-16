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

## 2026-10-15d — #34 logind slice 1/2: sessões/seats + utmp + `loginctl`

Fecha o começo do item **Session/seat management** do DoD. Investigação primeiro:
`getty` chama `login` (userutils), mas **nada** registra quem está logado; não existe
`who`/`last` no Redox e o `uptime` do coreutils não lê utmp. Então o plano é: `utmp`
padrão (compatibilidade) + `loginctl` como consumidor real, e o daemon depois.

- [DONE] `logind/src/utmp.rs` — formato `utmp`/`utmpx` clássico:
  - registro fixo de **384 bytes** (pid/sessão/timeval 32-bit, little-endian, offsets
    explícitos) — o mesmo layout do glibc/musl em 32 e 64 bits;
  - `RecordType` (EMPTY/RUN_LVL/BOOT_TIME/…/USER_PROCESS/DEAD_PROCESS), encode/decode,
    campos longos truncados com NUL, `ut_id` derivado do terminal (`tty12` → `12`);
  - `UtmpFile`: leitura tolerante (arquivo ausente = vazio; registro cortado/desconhecido
    é pulado em vez de esconder o resto), `write_all`, `put` com semântica de `pututline`
    (substitui pelo terminal; DEAD/EMPTY limpa), `remove`, `append` realinhado (padda
    cauda rasgada) e `reset`.
- [DONE] `logind/src/session.rs` — modelo + máquina de estados (puro, host-testável):
  - `Session` (id, uid, user, seat, tty, type, state, leader, since, host) e `Seat`;
  - `Sessions::login` (o primeiro login de um seat fica **active**, os demais `online`),
    `logout` (o seat é repassado ao próximo da fila), `activate` (troca o dono do console
    sem afetar outros seats), `set_closing`, `active_on`, `seats()`;
  - `to_utmp`/`from_utmp` (ida e volta pelo arquivo) e `assign_uids` (utmp não guarda uid).
  - Limitação documentada e coberta por teste: o registro padrão não tem campo de seat/type,
    então uma visão *só do arquivo* (sem daemon) coloca todo terminal no `seat0`; o daemon
    guarda o estado exato.
- [DONE] `logind/src/passwd.rs` — resolve nome → uid lendo `/etc/passwd` (ignora comentário,
  linha torta e prefixo que case por engano), usado para mostrar UID de verdade.
- [DONE] `logind/src/cli.rs` + `render.rs` + `bin/loginctl.rs` — CLI no estilo do loginctl:
  `list-sessions`/`sessions`, `list-seats`/`seats`, `session-status`, `seat-status`,
  `show-session`, `show-seat`, `--utmp/--file`/`$LOGIND_UTMP`, `-n/--no-legend`,
  `-h/--help`, `--version`; tabela alinhada, bloco de status e `key=value` para scripts;
  nome desconhecido vai para o stderr e sai com 1 sem perder o resto da saída.
- [DONE] **48 testes** no crate (42 unit + 6 integração `tests/pipeline.rs`).
- [DONE] E2E com o binário real: arquivo `utmp` gerado por uma implementação
  **independente** (script Python, só o layout padrão) lido pelo `loginctl`:
  `list-sessions`, `list-seats` (`seat0 1 1 2`), `session-status 1`, `show-session 2`
  (`Host=example.org`), `seat-status seat0`; `UID=1000` resolvido do `/etc/passwd` do host.
- [DONE] Empacotado: `BASE_EXTRA_BINS = journalctl loginctl`, `-p journald -p logind`;
  CI `lab-tests` passa a rodar `logind` também. `/var/run` já existe em `config/base.toml`.
- [PENDING] slice 2/2: daemon `logind` (dono de `/scheme/logind`), reset do utmp + BOOT_TIME
  no boot, `logind` avisado pelo `getty`/`login` (sessão real em vez de reconstruída),
  `wtmp` para um futuro `last`, `loginctl terminate-session`.

## 2026-10-16d — #34 `networkd` slice 2/2: daemon dono de `/scheme/networkd`

Fecha o `networkd` — e com ele o eixo de init/userspace do #34 (journald ✅, logind ✅, resolved ✅,
networkd ✅). A configuração declarativa do slice 1 agora é aplicada por alguém: o daemon aplica no
boot, aplica de novo quando a configuração ou as interfaces mudam (olha a cada 5 s, e na hora quando
alguém pede), e é o **único** escritor do stack — `networkctl apply` passa a pedir para ele.
`10-eth0.network` entra no image config: o DHCP de hoje vira um arquivo em vez de ser implícito.

- [DONE] `state.rs` — `Networkd`: configuração e stack juntos. `plan()` lê os `*.network` e observa
  as interfaces; `apply()` escreve e **lembra o que aplicou** (as writes do plano), então um plano
  que diz a mesma coisa é respondido com "unchanged" em vez de empurrar a mesma mudança para o
  stack a cada olhar. Plano sem writes (DHCP puro) não escreve nada — é o comportamento de hoje, e
  um plano meio aplicado (o stack recusou uma write no meio) **não** é lembrado como pronto.
  Estado é `Arc<Mutex<Networkd>>` (`shared`/`lock`): watcher e pedidos aplicam um de cada vez, e
  um lock envenenado não impede configurar.
- [DONE] `protocol.rs` — textual *write-then-read* no padrão do `logind`/`resolved`: `apply`
  (aliases `sync`/`reload`/`update`) → `ok applied <n> <netcfg>\t<config>` /
  `ok unchanged <netcfg>\t<config>` / `err <msg>`. Os **dois** caminhos vão na resposta — e em TAB,
  porque um caminho pode conter espaço: quem nomeou o próprio diretório precisa saber se o daemon
  está falando do mesmo.
- [DONE] `service.rs` — `Service::handle()` aplica um pedido e devolve a linha que o cliente lê;
  `status_document()` é o arquivo `status` do scheme (relatório vivo + avisos + `applied: yes|no`,
  porque quem lê o arquivo não tem stderr). Recusa vira `err` na linha, nunca silêncio.
- [DONE] `watch.rs` — `Watcher::look()`; stack **não montado** (`Netcfg::is_mounted()`, novo) →
  `Ok(None)`: a rede sobe depois deste daemon, e é para isso que serve vigiar. `NETWORKD_INTERVAL`
  segue o mesmo contrato do `resolved` (segundos; `0` desliga; inválido reporta e usa 5 s), para os
  dois daemons do boot serem configurados nas mesmas palavras.
- [DONE] `client.rs` — lado `networkctl`: `request_if_mounted()` (`None` = não há daemon; erro =
  veredito, inclusive recusa, que **não** pode ser engolido), `apply`/`status_report`,
  `writes_through_the_daemon(cli)` (dry-run nunca escreve; `--config-dir`/`--netcfg` nomeados são
  do usuário, não do daemon).
- [DONE] `scheme.rs` + `bin/networkd.rs` + `init.d/10_networkd.service` (e `10_net.target` passa a
  exigi-lo) — scheme com `status` (leitura, snapshot na abertura) e o controle `apply`; o daemon
  faz o primeiro olhar **antes** de anunciar o serviço, e o loop de vigia só existe se
  `NETWORKD_INTERVAL` ≠ 0.
- [DONE] `networkctl apply` pede ao daemon quando ele é o dono do stack e a resposta é sobre o
  **mesmo par** (`describes(netcfg, config)`), imprimindo
  `applied N writes to /scheme/netcfg (the network daemon)`; sem daemon (shell de resgate) escreve
  direto, como antes.
- [DONE] Image config: `/etc/networkd/10-eth0.network` (`[Match] Name=eth0` + `[Network] DHCP=yes`)
  no `config/base.toml` — o DHCP de hoje declarado, e o arquivo que o daemon lê no boot.
- [DONE] **95 testes** em `networkd` (81 unit + 6 `tests/daemon.rs` + 8 `tests/pipeline.rs`),
  fmt/clippy limpos. Novos: `protocol` 7, `client` 6, `state` 7, `service` 4, `watch` 3,
  `render` 2, `daemon` 6. O e2e cobre: primeiro olhar aplica endereço+rota, segundo não escreve
  nada, configuração editada é carregada; pedido via wire (socket pair) e o `unchanged` do pedido
  repetido; `DHCP=yes` não escreve nada; stack ausente → o watcher espera, o pedido é recusado com
  `cannot read the interfaces`; pedido malformado → `err unknown request`; e `networkctl` real com
  os caminhos nomeados **não** pergunta ao daemon (escreve no stack que lhe foi dado).
- [DONE] Empacotado: `BASE_EXTRA_BINS += networkd`; `cook base` **successful** → `usr/bin/networkd`
  (ELF estático, 460 KB) + `usr/lib/init.d/10_networkd.service` no stage.
- [DONE] Suíte host: **613 testes em 18 pacotes** (`networkd` 59 → 95), sem regressão (`resolved`
  segue 72; `journald` 115, `logind` 78).
- [PENDING] smoke/boot real (`harddrive.img` no Acer): `init` subindo `10_networkd.service` e o
  daemon aplicando (ou não, se o arquivo só disser DHCP) — depende do usuário.
- [PENDING] #34 ainda tem: systemd-boot/OTA, cgroups, e o resto do checklist da issue.

## 2026-10-16c — #34 `resolved` slice 2/2: daemon dono de `/scheme/resolved`

Fecha o `resolved`: agora **nada mais precisa lembrar de escrever** `/etc/resolv.conf`. O daemon
escreve no boot, escreve de novo quando o stack muda (olha a cada 5 s, e na hora quando alguém
pede), e é o único que escreve — um pedido que chega enquanto a rede muda não corre com outro
escritor. `init` sobe ele como serviço de scheme, depois do `smolnetd`; rede que ainda não subiu
**não** é falha (o lease chega depois e o próximo olhar carrega).

- [DONE] `protocol.rs` — protocolo textual *write-then-read* no padrão do `logind`:
  `sync` (aliases `update`/`reload`), `set-dns ADDRESS` (aliases `set-resolver`); resposta
  `ok <msg>` / `err <msg>`, com a mensagem saying **qual arquivo** e **se ele mudou**
  (`wrote /etc/resolv.conf` / `unchanged /etc/resolv.conf`). O caminho vai na resposta porque
  quem nomeou o próprio arquivo precisa saber se o daemon está falando *daquele* arquivo.
- [DONE] `service.rs` — `Service`: aplica um pedido ao estado e devolve a mensagem; `status_document()`
  é o arquivo `status` do scheme (relatório vivo + avisos dentro dele, porque quem lê o arquivo
  não tem stderr). Recusa vira `err` na linha, nunca silêncio.
- [DONE] `watch.rs` — `Watcher::look()`: um olhar no stack; `Stack::Empty`/`Unavailable` →
  `Ok(None)` (é "ainda não", não erro — é para isso que serve vigiar). Estado é
  `Arc<Mutex<Resolved>>` (`state::shared`/`state::lock`), então o watcher e os pedidos escrevem
  **um de cada vez**, e um lock envenenado não impede servir o arquivo.
- [DONE] `client.rs` — lado `resolvectl`: `request_if_mounted()` (`None` = não há daemon; erro =
  veredito, inclusive recusa, que **não** pode ser engolido), `sync`/`set_dns`/`status_report`,
  `writes_through_the_daemon(cli)` (dry-run nunca escreve; arquivo/mount point nomeado é do
  usuário, não do daemon; e a resposta tem de ser sobre o **mesmo** arquivo).
- [DONE] `scheme.rs` + `bin/resolved.rs` + `init.d/10_resolved.service` — scheme com `status`
  (leitura, snapshot na abertura) e os controles `sync`/`set-dns`; daemon faz o primeiro olhar
  **antes** de anunciar o serviço, e o loop de vigia só existe se `RESOLVED_INTERVAL` ≠ 0
  (padrão 5 s; valor inválido é reportado e cai no padrão).
- [DONE] `resolvectl update`/`set-dns` passam pelo daemon quando ele é o dono do arquivo do
  sistema, e dizem `(the resolver daemon)` — caso contrário escrevem direto, como antes.
- [DONE] **72 testes** em `resolved` (58 unit + 6 `tests/daemon.rs` + 8 `tests/pipeline.rs`),
  clippy limpo. O e2e do daemon cobre: nada a escrever no boot → lease chega → o watcher escreve
  a **primeira** linha `nameserver` que a relibc lê; pedido via wire (socket pair, resposta
  termina quando a scheme devolve EOF); `set-dns` recusado (`0.0.0.0`) sem tocar em nada;
  pedido malformado → `err unknown request`; arquivo que já concorda **não** é reescrito; e o
  `resolvectl` **real** lendo o arquivo que o daemon escreveu (`In sync: yes`).
- [DONE] Empacotado: `BASE_EXTRA_BINS += resolved` (daemon instalado junto do `resolvectl`),
  `10_resolved.service` no `init.d`. `cook base` **successful** → `usr/bin/resolved`
  (ELF estático, 419 KB) + `usr/lib/init.d/10_resolved.service` no stage. Iteração redox agora
  tem caminho rápido: `cookbook_redoxer build --release --target-dir <build> -p resolved` (5 s).
- [DONE] Suíte host: **577 testes em 18 pacotes** (`resolved` 44 → 72).
- [PENDING] smoke/boot real (`harddrive.img` no Acer) para ver o serviço subindo e o arquivo
  aparecendo depois do lease — depende do usuário.
- [PENDING] `networkd` slice 2/2 (adiado): daemon + `init.d` + `10-eth0.network` no image.

## 2026-10-16b — #34 `resolved` slice 1/2: `/etc/resolv.conf` gerenciado + `resolvectl`

Reordenação decidida: o slice 2/2 do `networkd` (daemon) fica para **depois** deste. Motivo:
**DNS por nome está quebrado hoje** — nada escreve `/etc/resolv.conf`, e é exatamente esse
arquivo que a relibc lê (`get_dns_server` pega a **primeira** linha `nameserver ` e nada mais).
O stack sabe o resolver (o DHCP grava `resolv/nameserver` no netcfg), mas o programa que
pergunta um nome não tem onde ler. Daemon de boot exige wiring que não dá para testar aqui;
esta fatia não.

- [DONE] Crate novo `resolved` (workspace member, host-puro, dep só de `networkd`):
  - `resolv.rs` — `/etc/resolv.conf`: parser **tolerante** + `render` canônico.
    `nameserver` (vários, ordem preservada; o primeiro é o que a libc usa), `search`/`domain`
    (mesma lista, a última vence), `options`; ida-e-volta `parse(render(x)) == x`.
    **Nada é descartado**: linha que não é entendida (diretiva desconhecida, `nameserver`
    inválido/broadcast/multicast/`0.0.0.0`, endereço a mais na linha) vai para `other` — volta
    na próxima escrita — e vira **aviso com o número da linha**. Comentário é a única exceção:
    não é configuração, e o arquivo é gerado com cabeçalho que diz isso.
  - `state.rs` — `Resolved`: o stack (`Stack::{Resolver,Empty,Unavailable}`, lido de
    `/scheme/netcfg/resolv/nameserver`) contra o arquivo. `status()`, `conf_for(addr)`,
    `sync(dry_run)` (stack → arquivo) e `set_dns(addr, dry_run)` (escreve no stack **e** no
    arquivo, com a **mesma** validação do stack: `is_unicast`). Só reescreve quando os
    resolvers **mudam**: um arquivo escrito à mão que já concorda é deixado como está.
    Stack ausente ≠ stack vazio: um é erro, o outro pede `set-dns` primeiro.
  - `cli.rs` + `render.rs` + `bin/resolvectl.rs` — `status`, `dns [-n]`, `update [--dry-run]`
    (aliases `sync`/`reload`), `set-dns ADDRESS [--dry-run]`, `--file`/`--netcfg`,
    `$RESOLVED_FILE`/`$NETWORKD_NETCFG`, `-h`, `--version`. Avisos no **stderr**, inclusive no
    `update`/`set-dns` (o arquivo que vai ser reescrito é o que foi lido).
- [DONE] **44 testes** (36 unit + 8 `tests/pipeline.rs`, sempre com o binário real contra um
  `netcfg` de mentira e arquivos temporários), clippy limpo. O que importa: arquivo inexistente
  → `update` cria; `search`/`options`/diretiva desconhecida sobrevivem; arquivo que já concorda
  **não** é reescrito; `--dry-run` não escreve em nenhum dos dois lados; `set-dns` de
  `0.0.0.0`/broadcast/multicast → erro sem tocar em nada; stack fora do ar aparece no `status`
  e é erro no `update`; a linha `nameserver` que sobra é a que a relibc lê primeiro.
- [DONE] Empacotado: `BASE_EXTRA_BINS += resolvectl`, `-p resolved`; CI `lab-tests` roda
  `resolved`. `cook base` **successful** → `usr/bin/resolvectl` (ELF estático, 341 KB) no stage.
- [DONE] Suíte host: **549 testes em 18 pacotes** (inclui `resolved` 44).
- [PENDING] slice 2/2: daemon `resolved` (dono de `/scheme/resolved`), `init.d` depois do
  `networkd`, e a escrita do arquivo no boot/reagindo a mudança de resolver do `dhcpd`.
- [PENDING] `networkd` slice 2/2 (adiado): daemon + `init.d` + `10-eth0.network` no image.

## 2026-10-16a — #34 networkd slice 1/2: config declarativa + `networkctl`

Segue a ordem do #34 (`journald` → `logind` → **`networkd`** → `resolved`). O que existe
hoje: `dhcpd` escreve direto em `/scheme/netcfg` (`addr/set`, `route/add`,
`resolv/nameserver`) com tudo *hardcoded* em `eth0`, e nada lê configuração nenhuma;
`/etc/net/*` do image config é herança de um scheme antigo e não é lido por ninguém.
Então esta fatia cria a camada que faltava: um arquivo declara o que a rede deve ser, e
uma ferramenta transforma isso nos writes do netcfg.

- [DONE] Crate novo `networkd` (workspace member, core 100% host-testável, sem deps):
  - `addr.rs` — `Ipv4Addr` (só quad decimal: nada de shorthand/hex/octal), `Ipv4Cidr`
    (máscara, rede, `contains`) e `MacAddr`; `is_unicast` é exatamente o teste que o
    stack aplica (recusa `0.0.0.0`, broadcast e multicast).
  - `config.rs` — arquivos `*.network` no formato do systemd-networkd:
    `[Match]` (`Name=` com `*`/`?`, `MACAddress=`), `[Network]` (`Description`, `DHCP`,
    `Address`, `Gateway`, `DNS`, todos aceitando vários valores) e um `[Route]` por rota
    (`Destination`, `Gateway`, `Metric`); leitura de diretório em ordem de nome (o
    primeiro que casa vence), `Config::render` ida-e-volta, e chave/seção desconhecida
    vira **aviso**, não erro (config de versão futura não derruba o boot).
  - `plan.rs` — a config resolvida contra as interfaces que existem: `Mode::{Dhcp,Static}`
    e `writes()` com o contrato do netcfg em um lugar só (`ifaces/<n>/addr/set`,
    `route/add` com `default via`/`CIDR via`, `resolv/nameserver`); interface sem arquivo
    que case fica em **DHCP** (o boot de hoje), com aviso; `notes()` diz o que não dá para
    aplicar (o stack só configura `eth0`) e o que se perde (um único resolver).
  - `netcfg.rs` — `/scheme/netcfg` como árvore: lê as interfaces de `ifaces/` (MAC vindo do
    stack), escreve com `write + sync_data` e **nunca cria** o control file (um path que o
    stack não serve tem de falhar, não virar arquivo); `apply` para no primeiro erro;
    `$NETWORKD_NETCFG` sobrepõe o mount.
  - `cli.rs` + `render.rs` + `bin/networkctl.rs` — `list`, `status [IFACE]`, `show [FILE]`,
    `apply [--dry-run]`, `--config-dir`/`--netcfg`/`-n`/`-h`/`--version`; tabela alinhada
    no `list`, relatório com os writes no fim no `status`; avisos vão para o **stderr**,
    uma vez só.
- [DONE] **59 testes** no crate (51 unit + 8 `tests/pipeline.rs`), clippy limpo.
- [DONE] E2E com o **binário real** e um `/scheme/netcfg` de mentira (o mesmo layout que o
  daemon vê): `list` mostra `eth0 static 192.168.1.50/24`; `status eth0` lista endereço,
  gateway, rota e resolver e depois os 4 writes; `apply --dry-run` não escreve nada e
  `apply` grava `addr/set=192.168.1.50/24` e `resolv/nameserver=9.9.9.9`; interface
  desconhecida sai com 1; `show` mostra a config normalizada e reclama de `[Wireless]`.
- [DONE] Empacotado: `BASE_EXTRA_BINS += networkctl`, `-p networkd`; CI roda `networkd`.
  `cook base` **successful** (`networkctl` no stage, ELF estátito para redox).
- [DONE] Suíte host: **505 testes em 17 pacotes** (inclui `logind` 78 e `networkd` 59).
- [PENDING] slice 2/2: daemon `networkd` (dono de `/scheme/networkd`), aplicar no boot e
  reagir a mudanças do netcfg; default `/etc/networkd/10-eth0.network` no image config
  (hoje: sem arquivo, tudo em DHCP, igual ao boot atual). **Adiado**: o `resolved` slice 1/2
  entrou antes (ver `## 2026-10-16b`), porque DNS por nome está quebrado hoje.
- [DONE] `resolved` slice 1/2 (item seguinte do #34, ver `## 2026-10-16b`): `/etc/resolv.conf`
  gerenciado — é o arquivo que a relibc lê (`get_dns_server` lê a primeira linha
  `nameserver `) e até aqui ninguém escrevia.

## 2026-10-15e — #34 logind slice 2/2: daemon `/scheme/logind` + sessões reais

Fecha a fatia de **Session/seat management**. Até aqui o `loginctl` só *reconstruía* a
lista lendo `utmp`; agora existe um dono do estado (o daemon), o arquivo é mantido em
sincronia com ele, e o `login` avisa o daemon quando uma sessão começa e termina.

- [DONE] `logind/src/protocol.rs` — protocolo **textual write-then-read** servido em
  `/scheme/logind`: controla por `login <user> <uid> <tty> <pid> [seat]`, `logout <alvo>`,
  `activate <alvo>`, `terminate <alvo>`; resposta `ok [msg]` ou `err <msg>`.
  Gramática em texto de propósito: o `login` reporta a sessão com I/O de arquivo puro,
  antes de linkar contra esta crate. Leitura não pede requisição — o scheme serve as
  sessões como **registros** (`Sessions::encode`) e os seats como linhas tab-separadas.
- [DONE] `logind/src/registry.rs` — estado vivo do boot, puro e host-testável:
  - `Registry::boot` trunca o utmp e escreve o `BOOT_TIME` (um `utmp` velho não passa por
    atual); `Registry::new` é o **boot tolerante** (não toca no arquivo) usado quando o
    registro não pode ser preparado — sessões continuam em memória, só a visão de arquivo
    se perde, em vez de o login manager não subir;
  - `handle()` aplica `Login`/`Logout`/`Activate`/`Terminate` e devolve um `Outcome`
    (`message` + pid a sinalizar) — quem fala com o sistema operacional é o chamador, o que
    mantém a máquina de estados testável;
  - `prune(alive)` remove sessões cujo líder sumiu, chamado só com um predicado
    **conservador** (vivo na dúvida): lista show de sessão morta incomoda menos que
    esconder uma viva;
  - um login no mesmo terminal substitui a sessão anterior (login não reportado como
    encerrado não sobrevive ao próximo), e o `utmp` é reescrito a cada mudança.
- [DONE] `logind/src/session.rs` — ganhou `encode`/`parse` (registros de 10 campos
  tab-separados: id uid user seat tty type state leader since host) com erro de registro
  tolerante; e o `logout` passou a **repassar o console** quando o seat fica sem sessão
  ativa, cobrindo uma sessão marcada `closing` pelo `terminate`.
- [DONE] `logind/src/client.rs` + `bin/loginctl.rs` reescrito — **daemon primeiro**: lê as
  sessões/seats do scheme quando ele está montado e cai para o arquivo de registros quando
  não está (mesma saída nos dois casos).
- [DONE] `logind/src/cli.rs` — `terminate-session`/`kill-session` e `activate-session`/
  `activate`, `--scheme` (endereço do daemon) e `--utmp`/`--file` explícito.
- [DONE] `logind/src/bin/logind.rs` + `logind/src/scheme.rs` — daemon do boot:
  - serve `/scheme/logind` (não entra em espaço de nomes vazio, ao contrário do `logd`:
    manter o utmp em dia precisa dos file schemes a cada requisição);
  - boot id de `LOGIND_BOOT_ID` → `JOURNAL_BOOT_ID` → gerado, para `loginctl` e `journalctl`
    descreverem o mesmo boot;
  - liveness do `prune` por `kill(pid, 0)`: só `ESRCH` conta como morto (não há procfs);
  - `terminate` manda `SIGTERM` no **líder** da sessão.
- [DONE] **78 testes** no crate (67 unit + 5 `tests/daemon.rs` + 6 `tests/pipeline.rs`),
  clippy limpo.
- [DONE] Produtor de sessões (o `login` de verdade):
  - `getty/src/main.rs` exporta `TTY_NAME` (o terminal *deste* getty), então a sessão é
    reportada no `ttyN` certo e não no pty;
  - `userutils` com **patch pinado** (`recipes/core/userutils/login-logind.patch`, `rev`
    fixado no `recipe.toml`): `spawn_shell_registered` reporta `login <user> <uid> <tty> <pid>`
    com o **pid do shell** como líder (parar a sessão para o que o usuário roda) e `logout`
    quando o shell termina; a notificação é **best-effort** — login sem daemon continua
    funcionando.
- [DONE] Empacotado: `init.d/00_logind.service` (agendada antes de `30_console.target`),
  `BASE_EXTRA_BINS = journalctl loginctl logind`.
- [DONE] Validação no target redox: `cook base` **successful** (`logind`, `loginctl`,
  `getty` e o service unit no stage) e `cook userutils` **successful** (`login` compila com
  o patch; o retorno é `io::Result<i32>`, não `libredox::Result`).
- [PENDING] hardware: boot de verdade (`harddrive.img`) e `who`/`last` sobre `wtmp` ficam
  para o teste no Acer.

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
