# Treasury ROAR Dice

Le contrat ne garde pas le rake pour l’owner. Le rake reste dans le **bankroll** jusqu’au buffer (`2 × min_bankroll`). Au-delà, `skimToTreasury` envoie le surplus à `treasury`.

## Règle

| Rôle | Wallet | Rôle |
|---|---|---|
| Owner | PEM de deploy | pause, seed, `setTreasury`, `startRound` |
| Treasury | wallet ops PrideVault | reçoit EGLD + ROAR skimmés |

**Séparer owner et treasury.** Si le PEM owner fuit, on change la treasury sans perdre le bankroll.

La treasury n’est pas un smart contract obligatoire : une adresse `erd1…` standard suffit (celle qui paie déjà le buyback / ops PrideVault).

## Config avant deploy

Dans `deploy/devnet.env` :

```bash
TREASURY=erd1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

`init` écrit cette adresse on-chain. View : `getTreasury`.

## Changer après deploy

```bash
chmod +x scripts/set-treasury.sh
./scripts/set-treasury.sh erd1...
```

Appelle `setTreasury` (owner only). Vérifier :

```bash
mxpy contract query $SC --function getTreasury --proxy $PROXY
```

## Skim

N’importe qui peut appeler `skimToTreasury` quand :
- aucun round ouvert
- resolve terminé
- free bankroll > `2 × min`

Sinon l’appel no-op (rien envoyé).

## PrideVault

`src/lib/sections.ts` :

```ts
export const GAMES = {
  roarDice: "",      // SC après deploy
  treasury: "",      // même adresse que TREASURY on-chain
  rakeEgldBps: 400,
  rakeRoarBps: 200,
  houseEdgeBps: 500,
}
```

Colle ton adresse ops ici. Sans adresse réelle, le deploy refuse (`treasury required`).
