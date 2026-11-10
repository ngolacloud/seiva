# ADR 0022 — Dados de seed e demo por módulo em CSV

- Estado: Aceite
- Data: 2026-11-09
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0002, ADR 0003, ADR 0021

## Contexto

Um projecto precisa de dois tipos de dados pré-carregados: **dados de
referência/seed** que todos os ambientes têm de ter (tabelas de referência,
registos por omissão), e **dados de demo** com que um programador sobe um serviço
para o ver a funcionar. O Odoo resolve isto com `data/` (sempre carregado) e
`demo/` (só em dev) por addon, em CSV, carregados pelo ORM para as restrições
dispararem e cada linha ter um *external id* estável para idempotência e
referências cruzadas.

As restrições do Seiva moldam a adaptação:

- Um **módulo são cinco camadas e é dono dos seus dados** (ADR 0003). O seed de um
  módulo pertence *ao módulo*, para que o `seiva module split` o leve para o
  serviço autónomo.
- **Cada caso de uso é uma transacção e os módulos cooperam por eventos** (ADR
  0021). Carregar linhas por uma facade publicaria eventos de domínio e dispararia
  a coreografia — desejável para *demo*, errado para *dados de referência* em massa.
- O build por omissão é **sem DORM e corre sobre adaptadores in-memory**, sem base
  de dados e sem dependências novas (o gate de cadeia de fornecimento `cargo deny`).
- Uma imagem distroless **não tem árvore de código em runtime**, logo os CSV têm de
  viajar no binário, não lidos do disco.
- Os ids são `Id([u8;16])` de 128 bits, cunhados no create; **não há conceito de
  external id** que torne um carregamento idempotente.

## Decisão

1. **Pastas CSV por módulo.** Um módulo inclui `modules/<name>/seed/*.csv`
   (referência) e `modules/<name>/demo/*.csv` (demonstração). Ficam sob
   `modules/<name>/`, para que o `seiva module split` os leve para
   `apps/<name>-service` sem alterações.

2. **Carregamento híbrido.**
   - O **seed** carrega em **cada arranque**, pela **data-layer** com **upsert
     idempotente**, e **não publica eventos**. É dado de referência, não uma
     transacção de negócio.
   - A **demo** carrega **só em dev** (`AppEnv::is_development()`, ou um
     `SEIVA_LOAD_DEMO` explícito), **pela facade do módulo**, para cada linha correr
     o caso de uso real — as invariantes de domínio, e o evento de domínio onde a
     facade o publica (para uma demo poder exercitar a coreografia após um `drain`).

3. **External id determinístico, sem tabela de mapeamento.** A 1ª coluna `id` do
   CSV é uma chave textual estável. O id da entidade é
   `seed::external_id("<module>.<resource>", key)` — FNV-1a sobre o âmbito, um
   separador e a chave, 128 bits, aritmética inteira pura. A mesma chave dá sempre o
   mesmo id (logo um reseed faz upsert em vez de duplicar) e a linha de um módulo
   pode ser referenciada por outro por chave. É o external id do Odoo sem a tabela
   de mapeamento `ir.model.data`.

4. **Registo em tempo de compilação, aplicado na composição-raiz.** Cada CSV é
   embebido com `include_str!` e declarado com `register_seed!`
   (`crates/delonix-seiva/src/seed.rs`), espelhando o registo de admin (ADR 0008):
   `registered_seeds()` torna as fontes descobríveis por uma fitness test e pelo
   `seiva db seed`. A *aplicação* (upsert / chamada à facade) é fiada na
   composição-raiz `apps/<name>-api`, o único sítio que segura os adaptadores
   concretos — tal como as fusões de router dos módulos. Um módulo gerado por
   `seiva module new` produz: um repositório com chave por id (logo `save` é um
   upsert idempotente), `infrastructure::load_seed`, `interface::load_demo`, ambos
   os CSV, e o passo de arranque no `main`.

5. **Um parser CSV sem dependências.** `seed::parse_csv` é um subconjunto do RFC
   4180 (linha de cabeçalho, campos com aspas e `""` como escape, comentários `#`),
   para o build por omissão não ganhar superfície de cadeia de fornecimento.

6. **`seiva db seed [--demo]`.** Corre a composição-raiz com `SEIVA_SEED=1` (e
   `SEIVA_LOAD_DEMO=1` para `--demo`): os módulos aplicam o seed e o binário sai
   antes de servir — reaproveitando a única fiação de arranque (como o `SEIVA_EMIT`
   faz para o `seiva openapi`).

## Consequências

- O seed corre em cada arranque e é idempotente, logo um reinício nunca duplica
  dados de referência. A persistência é a do adaptador: o adaptador in-memory por
  omissão torna o seed efémero (um smoke test); um adaptador DORM persiste-o.
- Um projecto acabado de gerar fica sem avisos: o gate de dev (`let load_demo`) é
  inserido com o primeiro módulo, não no esqueleto vazio.
- O repositório gerado passa a ter chave por id — um adaptador in-memory
  estritamente melhor (o `Vec` anterior acrescentava, o que duplicaria num reseed).
- As fitness functions provam a forma: `generated_module_has_seed_and_demo`,
  `composition_root_wires_seed_on_boot`, e o já existente
  `generated_project_compiles_standalone` (que agora liga o módulo ao `main`, logo
  o caminho de seed no arranque é mesmo compilado).
- A demo pela facade só dispara eventos onde a facade já publica um; o caso de uso
  gerado pelo scaffold ainda não abre transacção nem publica (achado F2/P0), por
  isso a coreografia de demo mais rica chega quando isso fechar.

## Alternativas consideradas

- **`data/{seed,demo}/` ao nível do projecto.** Rejeitada: quebra a propriedade de
  dados por módulo e o `split` teria de mover ficheiros.
- **Seed também pela facade (caminho uniforme).** Rejeitada: dados de referência em
  massa a disparar eventos de domínio accionariam a coreografia (semear orders
  reservaria stock e capturaria pagamentos). Seed não é transacção de negócio.
- **Carregar por SQL cru / insert DORM directo a contornar o domínio.** Rejeitada
  para *demo* (tem de exercitar invariantes); aceite em espírito para *seed*, que
  faz upsert de uma entidade de domínio construída pelo porto de repositório
  (ainda via `Record::new`, logo a invariante vale), nunca SQL cru.
- **O crate `csv`.** Rejeitada por agora, para o build por omissão ficar sem
  dependências e o gate `cargo deny` verde; o parser é um pequeno subconjunto do
  RFC 4180. Saída documentada: adoptar `csv` se alguma vez for preciso quoting rico.
- **Uma tabela de mapeamento `ir.model.data` (a abordagem do Odoo).** Rejeitada por
  desnecessária: um hash determinístico da chave externa dá idempotência e
  referências cruzadas sem tabela para guardar, migrar ou manter consistente.
