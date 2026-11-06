# Referência da API

Descrições orientadas à informação, exaustivas. Recorre a estas quando sabes o
que queres e precisas da forma exacta.

- [A CLI `seiva`](cli.md) — cada comando e flag
- [Definições e ambiente](settings.md) — cada variável de configuração
- [O descritor de contrato](contract-descriptor.md) — `Facade`, `FacadeDescriptor`,
  `MethodDescriptor`, `Param`, e o inventário em tempo de link
- [Wire schema](wire-schema.md) — o trait `WireSchema`, o derive, o inventário de
  esquemas
- [Os crates](crates/index.md) — o que cada crate `seiva-*` / `delonix-seiva` detém

A API completa ao nível dos tipos é gerada por `cargo doc --workspace --no-deps`
(cada item público está documentado; o rustdoc é a fonte da verdade para as
assinaturas).

---

*Parte da [documentação do Seiva](../index.md).*
