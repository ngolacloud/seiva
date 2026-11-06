# Caching

Faz caching deliberadamente, na camada que detém a invalidação.

## Caching de consultas (camada de aplicação)

O mediador CQRS ([CQRS](cqrs.md)) suporta **middleware tipado de cache de
consultas**: o resultado de um handler de consulta é posto em cache, com chave no
valor da consulta, e o middleware detém o seu tempo de vida. Esta é a costura de
caching de primeira classe — faz cache de *leituras de caso de uso*, onde o módulo
sabe quando um resultado está obsoleto (foi ele que escreveu a alteração), pelo que
a invalidação é correcta em vez de adivinhada.

## Caching HTTP (transporte)

Para `GET`s públicos e cacheáveis podes definir cabeçalhos de caching HTTP
(`Cache-Control`, `ETag`) na resposta, para que clientes e proxies os ponham em
cache. Usa isto apenas para respostas genuinamente seguras de pôr em cache; as
predefinições do framework não fazem cache.

## O que não fazer

Não faças cache por trás da camada de dados às cegas (uma cache global de leitura
directa) — é aí que vivem os bugs de dados obsoletos, porque a cache não sabe quando
uma escrita invalidou uma entrada. Faz cache onde a escrita acontece (a camada de
aplicação), ou na borda para respostas imutáveis/públicas.

> Âmbito: o middleware de cache de consultas do CQRS é o caminho suportado hoje;
> backends de cache mais ricos e auxiliares de cache HTTP são uma área que cresce
> com o uso real.

## Relacionado

- [Despacho CQRS](cqrs.md) · [Desempenho e cloud-native](performance.md)

---

*Parte da [documentação do Seiva](../index.md).*
