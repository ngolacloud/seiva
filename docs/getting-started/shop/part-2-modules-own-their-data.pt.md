# Parte 2: Os módulos possuem os seus dados

Cada módulo expõe **um contrato** — um trait `#[seiva::facade]` cujos métodos
declaram, cada um, a permissão que um chamador tem de possuir. As rotas HTTP, o
OpenAPI e as verificações de permissão são todos derivados dele. Aqui estão os
cinco, exactamente como aparecem em
[`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop).

## catalog — produtos

```rust
#[facade(version = "v1")]
pub trait Catalog {
    #[requires("catalog.manage")]
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, String>; // catalog.product_added

    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ProductDto>, String>;

    #[requires("catalog.read")]
    async fn get(&self, id: ProductId) -> Result<ProductDto, String>;
}
// CreateProduct { name, price_cents }   ProductDto { id, name, price_cents }
```

A regra de domínio vive em `catalog-domain`: o nome de um produto não pode estar
vazio e o seu `price_cents` tem de ser maior que zero (`Product::new`).

## inventory — níveis de stock

```rust
#[facade(version = "v1")]
pub trait Inventory {
    #[requires("inventory.manage")]
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, String>;  // inventory.received

    #[requires("inventory.manage")]
    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, String>;       // inventory.reserved

    #[requires("inventory.read")]
    async fn available(&self, id: ProductId) -> Result<StockDto, String>;
}
// ReceiveStock { product_id, qty }   Reserve { product_id, qty }   StockDto { product_id, available }
```

`reserve` falha com um erro `InsufficientStock` quando a quantidade disponível está
abaixo do que é pedido — uma regra de domínio, não uma restrição de base de dados.

## orders — encomendas

```rust
#[facade(version = "v1")]
pub trait Orders {
    #[requires("orders.manage")]
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, String>;  // orders.placed

    #[requires("orders.read")]
    async fn get(&self, id: OrderId) -> Result<OrderDto, String>;

    #[requires("orders.read")]
    async fn list(&self) -> Result<Vec<OrderDto>, String>;
}
// PlaceOrder { customer, lines: Vec<LineDto> }   LineDto { product_id, qty }
// OrderDto { id, customer, lines, status }   // status starts at "placed"
```

Repara que `lines` é um `Vec<LineDto>` — um DTO de wire aninhado. Como `LineDto`
deriva `WireSchema`, a forma real da encomenda (uma lista de itens de linha) é
publicada no OpenAPI sem uma spec escrita à mão. Uma encomenda tem de ter pelo
menos uma linha e cada linha uma quantidade positiva.

## payments — pagamentos

```rust
#[facade(version = "v1")]
pub trait Payments {
    #[requires("payments.charge")]
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, String>; // payments.captured | payments.declined

    #[requires("payments.read")]
    async fn get(&self, id: PaymentId) -> Result<PaymentDto, String>;
}
// Charge { order_id, amount_cents }   PaymentDto { id, order_id, amount_cents, status }
```

`charge` regista um pagamento `captured` ou `declined` (a demonstração recusa um
valor zero); ambos os desfechos fazem commit e emitem o seu evento. A política de
recusa é uma decisão de caso de uso, e o domínio mantém o invariante de que um
pagamento *captured* nunca pode ter um valor zero.

## notifications — mensagens enviadas

```rust
#[facade(version = "v1")]
pub trait Notifications {
    #[requires("notifications.send")]
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, String>; // notifications.sent

    #[requires("notifications.read")]
    async fn list(&self) -> Result<Vec<NotificationDto>, String>;
}
// SendNotification { to, subject, body }   NotificationDto { id, to, subject, body }
```

!!! note "Porquê `SendNotification`, não `Send`"
    O comando é `SendNotification` em vez do tentador `Send`: `Send` é o marker
    trait do Rust, e as futures das portas da camada de aplicação estão limitadas
    a `+ Send`, por isso um DTO chamado `Send` colidiria com o bound e falharia a
    compilar. Um pequeno lembrete de que os nomes dos teus DTOs partilham um
    namespace com a biblioteca padrão.

## Cinco contratos, sem acoplamento

Cada módulo nomeia apenas os seus próprios tipos. Nenhum deles importa o crate de
outro módulo. A parte seguinte mostra como, ainda assim, cooperam — através de
eventos, não de chamadas.

## A seguir

[Parte 3: Eventos entre módulos](part-3-events-between-modules.md).

---

*Parte da [documentação do Seiva](../../index.md).*
