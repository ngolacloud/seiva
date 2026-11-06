# Parte 2: Os módulos possuem os seus dados

Cada módulo expõe **um contrato** — um trait `#[seiva::facade]` cujos métodos
declaram cada um a permissão que o chamador tem de possuir. As rotas HTTP, o
OpenAPI e as verificações de permissão derivam todos dele. Aqui estão os cinco,
exactamente como aparecem em
[`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop).

## catalog — produtos

```rust
#[facade(version = "v1")]
pub trait Catalog {
    #[requires("catalog.manage")]
    async fn add_product(&self, cmd: CreateProduct) -> Result<ProductDto, CatalogError>; // catalog.product_added

    #[requires("catalog.read")]
    async fn list(&self) -> Result<Vec<ProductDto>, CatalogError>;

    #[requires("catalog.read")]
    async fn get(&self, id: ProductId) -> Result<ProductDto, CatalogError>;
}
// CreateProduct { name, price_cents }   ProductDto { id, name, price_cents }
```

A regra de domínio vive em `catalog-domain`: o nome de um produto tem de ser não
vazio e o seu `price_cents` tem de ser maior que zero (`Product::new`).

## inventory — níveis de stock

```rust
#[facade(version = "v1")]
pub trait Inventory {
    #[requires("inventory.manage")]
    async fn receive(&self, cmd: ReceiveStock) -> Result<StockDto, InventoryError>;  // inventory.received

    #[requires("inventory.manage")]
    async fn reserve(&self, cmd: Reserve) -> Result<StockDto, InventoryError>;       // inventory.reserved

    #[requires("inventory.read")]
    async fn available(&self, id: ProductId) -> Result<StockDto, InventoryError>;
}
// ReceiveStock { product_id, qty }   Reserve { product_id, qty }   StockDto { product_id, available }
```

`reserve` falha com um erro `InsufficientStock` quando a quantidade disponível é
inferior à que é pedida — uma regra de domínio, não uma restrição de base de
dados.

## orders — encomendas

```rust
#[facade(version = "v1")]
pub trait Orders {
    #[requires("orders.manage")]
    async fn place(&self, cmd: PlaceOrder) -> Result<OrderDto, OrdersError>;  // orders.placed

    #[requires("orders.read")]
    async fn get(&self, id: OrderId) -> Result<OrderDto, OrdersError>;

    #[requires("orders.read")]
    async fn list(&self) -> Result<Vec<OrderDto>, OrdersError>;
}
// PlaceOrder { customer, lines: Vec<LineDto> }   LineDto { product_id, qty }
// OrderDto { id, customer, lines, status }   // status starts at "placed"
```

Repara que `lines` é um `Vec<LineDto>` — um DTO de fio aninhado. Como `LineDto`
deriva `WireSchema`, o formato real da encomenda (uma lista de itens de linha) é
publicado no OpenAPI sem especificação escrita à mão. Uma encomenda tem de ter
pelo menos uma linha e cada linha uma quantidade positiva.

## payments — pagamentos

```rust
#[facade(version = "v1")]
pub trait Payments {
    #[requires("payments.charge")]
    async fn charge(&self, cmd: Charge) -> Result<PaymentDto, PaymentsError>; // payments.captured | payments.declined

    #[requires("payments.read")]
    async fn get(&self, id: PaymentId) -> Result<PaymentDto, PaymentsError>;
}
// Charge { order_id, amount_cents }   PaymentDto { id, order_id, amount_cents, status }
```

`charge` regista um pagamento `captured` ou `declined` (a demo recusa um montante
zero); ambos os desfechos confirmam e emitem o seu evento. A política de recusa é
uma decisão do caso de uso, e o domínio mantém o invariante de que um pagamento
*captured* nunca pode ter um montante zero.

## notifications — mensagens enviadas

```rust
#[facade(version = "v1")]
pub trait Notifications {
    #[requires("notifications.send")]
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, NotificationsError>; // notifications.sent

    #[requires("notifications.read")]
    async fn list(&self) -> Result<Vec<NotificationDto>, NotificationsError>;
}
// SendNotification { to, subject, body }   NotificationDto { id, to, subject, body }
```

!!! note "Porquê `SendNotification`, não `Send`"
    O comando é `SendNotification` em vez do tentador `Send`: `Send` é o trait
    marcador do Rust, e os futures de porta da camada de aplicação são limitados
    por `+ Send`, por isso um DTO chamado `Send` colidiria com o limite e falharia
    a compilação. Um pequeno lembrete de que os nomes dos teus DTOs partilham um
    namespace com a biblioteca padrão.

## Cinco contratos, sem acoplamento

Cada módulo nomeia apenas os seus próprios tipos. Nenhum deles importa o crate de
outro módulo. A próxima parte mostra como ainda assim cooperam — através de
eventos, não de chamadas.

## Seguinte

[Parte 3: Eventos entre módulos](part-3-events-between-modules.md).

---

*Parte da [documentação do Seiva](../../index.md).*
