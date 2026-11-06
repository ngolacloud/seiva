# Perguntas frequentes

### O Seiva é uma web framework ou um ORM?

O Seiva é a **framework** — a camada de serviço, distribuição e domínio à volta de
uma fonte de dados. A fonte de dados por omissão é o **DORM** (o ORM da Delonix), um
produto independente, ligado apenas através da feature opcional `dorm`. O núcleo da
framework nunca depende do DORM.

### O que me dá o Seiva para além de routing e validação?

Uma camada de serviço fina pára em routing + validação + auto-OpenAPI (sem ORM, sem
migrações, sem sistema de auth, sem scaffolding, sem admin). O Seiva mantém a
experiência de programador de auto-OpenAPI + validação e acrescenta as baterias —
uma camada de dados opcional, um sistema de auth com JWT verificado, scaffolding de
projecto/módulo, segurança por omissão — como um **binário único, compilado e sem
GC**. Vê [Seiva num relance](../getting-started/seiva-at-a-glance.md) e o
[panorama competitivo](../competitive-landscape.md).

### Tenho de usar o DORM?

Não. A camada de application de cada módulo codifica contra *portas* só-de-traits.
Os adaptadores por omissão são em memória (sem DORM), e podes assentar um repositório
sobre qualquer fonte de dados externa. O DORM é o padrão de baterias incluídas, não
um aprisionamento.

### Porquê Rust, quando o Prisma acabou de sair *do* Rust?

O Prisma moveu o seu *cliente* para fora do Rust porque a **fronteira FFI
Rust↔JavaScript** era o estrangulamento — não o Rust em si. Um consumidor
Rust-nativo (DORM, Seiva) não tem tal fronteira: sem GC, sem FFI, segurança em tempo
de compilação até ao binário, implantação de binário único. Vê
[a camada de dados](../topics/data-layer/index.md).

### Existe uma consola de admin embutida?

Ainda não — um admin auto-CRUD completo é um item de roadmap. É uma das lacunas
apontadas no [panorama competitivo](../competitive-landscape.md).

### O que funciona hoje vs o que está planeado?

O Seiva está em estágio fundacional. O [registo de engenharia](../progress.md) e os
[ADRs](../internals/adrs.md) seguem as capacidades com honestidade; as páginas
marcadas como "planeadas" nestes docs trazem o esboço pretendido.

### Onde reporto problemas / contribuo?

Vê [Contribuir](../internals/contributing.md). Uma mudança de arquitectura aterra
primeiro num [ADR](../internals/adrs.md).

---

*Parte da [documentação do Seiva](../index.md).*
