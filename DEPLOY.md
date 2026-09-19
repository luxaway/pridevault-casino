# Déploiement ROAR Dice

**Devnet seulement pour v1.** Pas de bankroll mainnet sans audit tiers.

## 0. Outils

```bash
cargo install multiversx-sc-meta --locked
pip install multiversx-sdk-cli   # mxpy
```

Wallet deployer : PEM ou keystore. Faucet devnet : https://r3d4.fr/faucet

## 1. Build

```bash
cd pridevault-casino
sc-meta all build
ls output/pridevault-casino.wasm
```

## 2. Config

```bash
cp deploy/devnet.env.example deploy/devnet.env
```

Remplir :
- `PEM` — chemin vers le wallet owner
- `TREASURY` — wallet PrideVault qui reçoit le skim
- `ROAR_TOKEN` — ticker ESDT **sur la chaîne visée**
  - mainnet : `ROAR-e5185d`
  - devnet : token de test (le vrai ROAR n’existe pas sur D)

Seed recommandé (exemple dans le `.env`) :
- 10 EGLD + 5 000 ROAR de test dans le contrat
- min bankroll = la moitié du seed
- cap mise EGLD 0.2 / ROAR 500

## 3. Deploy

```bash
chmod +x scripts/deploy-devnet.sh
./scripts/deploy-devnet.sh
```

`init` attend, dans l’ordre :

| # | Arg |
|---|---|
| 1 | treasury address |
| 2 | ROAR token id |
| 3 | min bet EGLD |
| 4 | cap max EGLD |
| 5 | min bet ROAR |
| 6 | cap max ROAR |
| 7 | round blocks |
| 8 | min bankroll EGLD |
| 9 | min bankroll ROAR |

Note l’adresse `erd1qqqq…` du contrat.

## 4. Seed + premier round

```bash
SC=erd1qqqq...
mxpy contract call $SC --function fundBankroll --value $SEED_EGLD \
  --pem $PEM --proxy $PROXY --chain $CHAIN --recall-nonce --gas-limit 5000000 --send

mxpy contract call $SC --function fundBankroll \
  --token-transfers $ROAR_TOKEN $SEED_ROAR \
  --pem $PEM --proxy $PROXY --chain $CHAIN --recall-nonce --gas-limit 8000000 --send

mxpy contract call $SC --function startRound \
  --pem $PEM --proxy $PROXY --chain $CHAIN --recall-nonce --gas-limit 6000000 --send
```

## 5. Brancher PrideVault

Dans `PrideVault/src/lib/sections.ts` :

```ts
export const GAMES = {
  roarDice: "erd1qqqq...", // adresse du SC
  rakeEgldBps: 400,
  rakeRoarBps: 200,
  houseEdgeBps: 500,
} as const;
```

Le bouton **Miser** du desk Games s’active seulement si `GAMES.roarDice` n’est pas vide.

## 6. Checklist avant d’ouvrir

- [ ] wasm build OK
- [ ] deploy tx success sur l’explorer devnet
- [ ] `getRoarToken` / `getTreasury` views OK
- [ ] seed EGLD + ROAR arrivés (`getFreeBankrollEgld` / `Roar`)
- [ ] `startRound` ouvert
- [ ] 1 mise test + `resolveRound` + `claim`
- [ ] `skimToTreasury` refusé tant que le buffer 2× n’est pas plein
- [ ] adresse collée dans PrideVault

## Mainnet

Ne pas copier-coller ce script tel quel. Changer `CHAIN=1`, proxy mainnet, ROAR-e5185d, seed réel, audit.
