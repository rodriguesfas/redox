# fdt vendorizado + correção AIOS (tokens NOP)

## Por quê

O crate `fdt` (github.com/repnop/fdt) dessincroniza ao encontrar tokens
`NOP` (`FDT_NOP = 4`) no meio da lista de propriedades/subnós de um nó.

Firmwares (Raspberry Pi 4 / libfdt / u-boot) aplicam *fixups* no DTB com
`fdt set`/`fdt put`, e o resultado contém NOPs. Sem o fix:

- `Fdt::new` / `find_compatible` param de enxergar os nós depois do
  1º NOP → o **kernel ficava “mudo”** (não passava do early boot);
- o driver `bcm2835-sdhcid` não achava o nó `brcm,bcm2711-emmc2`.

## O que muda

Pular tokens `NOP` nos 5 pontos de `src/node.rs`:
`parse_property`/`children`, `find_node`, `all_nodes` (×2) e
`skip_current_node`. Nada além disso.

Diffs contra o upstream:

- `fdt-nop-tokens.patch` — vs git rev `2fb1409` (0.2.0-alpha1)
- `fdt-nop-tokens-0.1.5.patch` — vs crates.io `0.1.5`

## Cópias vendorizadas (usadas pelo build)

| Diretório | Origem | Consumidores |
|---|---|---|
| `git-0.2.0-alpha1/` | git rev `2fb1409` (patcheado) | kernel, bootloader e base (`[patch."https://github.com/repnop/fdt.git"]`) |
| `crates-io-0.1.5/` | crates.io 0.1.5 (patcheado) | base (`[patch.crates-io]`) |

Os `Cargo.toml` de `recipes/core/{kernel,bootloader,base}/source` apontam
para cá; o build não busca o fdt na rede.

> Obs.: o `bcm2835-sdhcid` (base) declarava o git rev `059bb23`. O vendor usa
> o `2fb1409`; a única diferença entre os dois revs é `src/pretty_print.rs`
> (não usado por esse consumidor), e ambos levam a mesma correção.

## Como reproduzir/atualizar

```sh
git clone https://github.com/repnop/fdt.git /tmp/fdt
git -C /tmp/fdt checkout 2fb1409edd1877c714a0aa36b6a7c5351004be54
patch -p1 -d /tmp/fdt < fdt-nop-tokens.patch
```
