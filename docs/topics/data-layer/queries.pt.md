# Fazer consultas

A camada `application` de um módulo consulta através da sua **porta de
repositório**, não de um handle global de base de dados — por isso o caso de uso
mantém-se testável e o SQL fica no adaptador (DORM).

## As consultas vivem por detrás de uma porta

A aplicação declara o que precisa como um trait; a camada `infrastructure`
implementa-o (em memória por omissão, suportado por DORM com a feature `dorm`):

```rust
pub trait RecordRepository {
    fn save(&self, record: &Record) -> impl Future<Output = PortResult<()>> + Send;
    // add the reads the use case needs: find_by_id, list, …
}
```

O caso de uso invoca a porta; nunca nomeia o DORM. É isso que permite aos testes de
unidade da Parte 5 correrem com um repositório falso e sem base de dados.

## Consultas tipadas através do DORM

O adaptador suportado por DORM constrói as suas leituras com o `QuerySet` tipado do
DORM — uma consulta verificada em tempo de compilação que o compilador puro baixa
para SQL do backend activo ([múltiplas bases de dados](multiple-databases.md)).
Exprimes a consulta em Rust; o DORM produz o dialecto.

## Paginação

Prefere a paginação por **keyset** (um cursor sobre uma ordenação estável) à
paginação por offset nas listas — é estável sob escritas e não degrada em páginas
profundas. A convenção da rota `list` (ver
[controladores e encaminhamento](../transport/controllers-and-routing.md)) carrega
os parâmetros da consulta; o repositório transforma-os numa leitura limitada e
ordenada.

## Sem N+1

O DORM resolve os dados relacionados com joins de SQL, não uma consulta por linha —
por isso carregar uma colecção com as suas relações é uma ida ao servidor, não N+1.
(Este é um ponto em que o DORM difere dos ORMs que historicamente buscavam as
relações em separado.)

## SQL cru, quando precisas

Para uma consulta que a camada tipada não exprime, desce à válvula de SQL cru
parametrizada — nunca concatenes valores em strings. Ver [SQL cru](raw-sql.md).

## Relacionado

- [Modelos e esquema](models.md) · [Transacções](transactions.md) · [SQL cru](raw-sql.md)
- [A camada de dados](index.md)

---

*Parte da [documentação do Seiva](../../index.md).*
