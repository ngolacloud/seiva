# A CLI e o Studio

O DORM entrega uma CLI instalável — o binário `dorm` — mais um administrador de
dados baseado em navegador, o **DORM Studio**, e uma REPL. Dentro de um projecto
Seiva alcanças sobretudo estes através dos invólucros `seiva db *` (vê
[Usar o DORM a partir do Seiva](using-from-seiva.md)), mas vale a pena conhecer a
ferramenta por baixo, porque `seiva db studio`/`pull` delegam directamente nela e
os outros comandos estão disponíveis no directório `data/` do teu projecto.

```sh
cargo install dorm-cli        # installs the `dorm` binary
dorm init blog                # scaffold a project
```

## Duas vias

Como o DORM é um ORM de tempo de compilação, os comandos vêm em duas vias:

- **Nativa** — correm dentro do binário instalado sobre um DSN, sem build do
  projecto: `doctor`, `db *`, `migrate status`, `migrate resolve`, `completions`,
  `init`, `new model`, `studio`, `format`, `db pull`.
- **Via Cargo** — compilam e correm um dos binários do próprio projecto, porque
  precisam dos modelos compilados: `validate`, `migrate dev`, `migrate deploy`,
  `migrate diff`, `push`, `shell`.

Comportamento global que vale a pena conhecer: `--dsn` sobrepõe o DSN de ambiente
para uma execução; `--json` em cada comando de leitura emite um objecto estável;
os códigos de saída são `0` ok, `1` erro, `2` uso, `3` verificações-encontraram-
problemas; um DSN nunca é impresso com a sua palavra-passe.

## A superfície de comandos

| Comando | Faz | Estado |
|---|---|---|
| `dorm init [name]` | Faz o scaffold de um projecto (lib + modelos + binários). | feito |
| `dorm new model <Name>` | Adiciona um modelo (struct + registo). | feito |
| `dorm generate` | Compila `.schema` → `src/models.rs`. | feito |
| `dorm validate` | Verifica os modelos quanto a erros com que uma consulta/migração tropeçaria (ao nível do esquema, depois ao nível do registo). | feito |
| `dorm format [--check]` | Imprime o esquema na sua forma canónica, sem perdas. | feito |
| `dorm doctor` | Diagnostica configuração, conectividade, versão do servidor, só-leitura, livro-razão. | feito |
| `dorm db push` / `dorm push` | Sincroniza o esquema com os modelos, **aditivamente** (nunca remove/retipa). | feito |
| `dorm migrate dev [name]` | Autoriza **e** aplica a próxima migração num só passo. | feito |
| `dorm migrate deploy` (= `dorm migrate`) | Aplica a cadeia revista por ordem sob um único lock. Idempotente. | feito |
| `dorm migrate status` | O livro-razão das versões aplicadas (`--json`). | feito |
| `dorm migrate diff` | Mostra as mudanças aditivas que o `push` aplicaria, em só-leitura. | feito |
| `dorm migrate resolve <version>` | Marca uma versão como aplicada sem correr o seu SQL — recuperação. | feito |
| `dorm db tables\|show\|count\|rows` | Percorre a base de dados viva, em só-leitura (`--json`; `rows --where col=value`). | feito |
| `dorm db execute "<SQL>" -p …` | Corre SQL explícito com parâmetros vinculados (só-leitura sem `--write`). | feito |
| `dorm db seed [--file]` | Corre um ficheiro de seed SQL (caminho de escrita; confirma primeiro). | feito |
| `dorm db shell` | Entrega-te ao cliente nativo (`psql`) sobre a base de dados configurada. | feito |
| `dorm shell [--write]` | A REPL do projecto (vê abaixo). | feito |
| `dorm studio [--write]` | O administrador de dados no navegador (vê abaixo). | feito |
| `dorm studio generate` | Escreve um binário de Studio **tipado** sobre os teus modelos. | feito |
| `dorm completions <shell>` | Imprime um script de autocompletamento. | feito |

> O DORM carrega também superfícies opcionais e geradas — um worker RAG, um
> servidor MCP — atrás de crates por adesão. Sob o Seiva estas responsabilidades
> sobem para dentro do framework (ADR 0054 do DORM): uma API REST, contas de
> utilizador, clientes de broker e os ciclos RAG/MCP de longa duração são a camada
> do framework, não a do ORM. Por isso não os conduzirás através de `dorm` num
> serviço Seiva; o framework é dono deles.

## A shell (REPL)

`dorm shell` abre uma REPL que lista os modelos que o linker reuniu e corre
consultas indexadas por string contra uma base de dados viva — a frente de
consulta dinâmica em pessoa. É só-leitura salvo se arrancada com `--write`.

```console
$ dorm shell
dorm> :models
dorm> Post title__icontains=rust limit=5
dorm> Post author__name__icontains=ada        # traverse a relation
```

Com `--write` também corre DML (`insert`/`update`/`delete`) através do compilador
e suporta **transacções interactivas** (`:begin`/`:commit`/`:rollback`), mais
`:sql` para pré-visualizar o SQL de uma consulta e `:raw` para a válvula crua
parametrizada. Cada chave de filtro é resolvida através do registo de modelos
antes de existir SQL algum, por isso uma coluna inventada é um erro, não uma
consulta. A crate `dorm-shell` também pode ser embebida.

## DORM Studio

O Studio é uma UI web sobre a tua base de dados viva: percorre, filtra, pesquisa,
ordena e pagina cada tabela, segue chaves estrangeiras em ambos os sentidos, e —
com `--write` — cria, edita e apaga linhas, tudo a partir do navegador. Vincula-se
**só a `127.0.0.1`**, por isso nunca é alcançável fora da máquina onde corre.

```sh
export DELONIX_ORM_DSN='host=/var/run/postgresql dbname=blog'
dorm studio --write          # omit --write to browse read-only; --port N to choose the port
#  open  http://127.0.0.1:5555
```

O Studio vem em dois sabores que partilham uma página e uma API JSON:

| | **Nativo** (`dorm studio`) | **Tipado** (`cargo run --bin studio`) |
|---|---|---|
| Aprende o esquema de | a base de dados viva (`information_schema`) | o registo que os teus modelos preenchem |
| Precisa de build do projecto | não — corre contra qualquer PostgreSQL | sim — liga os teus modelos |
| Conhece **nomes** de relação | não (só colunas de chave estrangeira) | sim (relações inversas também) |
| Conhece tipo/comprimento/unique declarado | do catálogo | do modelo |

Usa o Studio nativo para um olhar sem configuração a qualquer base de dados; usa o
Studio tipado (`dorm studio generate` escreve `src/bin/studio.rs`) dentro de um
projecto para obteres os nomes de relação dos teus próprios modelos, as relações
inversas e os tipos declarados.

O Studio mantém as garantias do ORM de uma ponta à outra: cada identificador é
validado contra o catálogo ou o registo antes de ser citado; cada valor é um
parâmetro vinculado; e um update ou delete sem filtro de chave é recusado, por
isso a grelha não consegue limpar uma tabela por um deslize. Abre em
**só-leitura** e só activa escritas com `--write` — a mesma
[protecção](transactions.md#read-only-mode) que a API tipada usa. Um apagamento em
massa corre dentro de uma transacção (todas as linhas seleccionadas vão ou nenhuma
vai), e a exportação CSV faz streaming da vista filtrada/ordenada actual (limitada
a 100 000 linhas).

> `seiva db studio` corre o Studio nativo por ti sobre o DSN do teu projecto; vê
> [Usar o DORM a partir do Seiva](using-from-seiva.md).

## Relacionado

- [Migrações](migrations.md) · [Transacções](transactions.md) ·
  [Backends](backends.md)
- [Usar o DORM a partir do Seiva](using-from-seiva.md) · a
  [referência da CLI `seiva`](../reference/cli.md)

---

*Parte da [documentação do Seiva](../index.md).*
