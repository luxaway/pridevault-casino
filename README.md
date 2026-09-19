# PrideVault Casino — ROAR Dice

Jeu on-chain MultiversX pour financer **PrideVault**.

Repo: https://github.com/luxaway/pridevault-casino

## Pourquoi ce jeu (et pas un flip 1-tx)

Sur MultiversX, un `roll` dans la même transaction que la mise est **tricheable** : le joueur simule la tx et ne l’envoie que s’il gagne.  
La doc officielle l’interdit.  
Ici : **rounds**. Les mises sont lockées *avant* le tirage. Le RNG ne tourne qu’à `resolveRound`, quand plus personne ne peut changer sa mise.

## Règles

- Dés 0–99
- Le joueur choisit `under` (2 à 96) : il gagne si `roll < under`
- House edge **4%** (400 bps) → trésorerie PrideVault
- Exemple : `under = 50` → chance 50% → payout ×1.92 (mise incluse)
- Bankroll dans le contrat, mise max limitée par la liquidité
- Gains en `claim` (pas de payout massif au resolve → moins de risque gas)

## Revenu projet

Sur le long terme, l’edge de 4% + les mises perdues restent dans le contrat.  
`skimToTreasury` envoie le surplus au-dessus de `min_bankroll` vers l’adresse trésorerie.

Ce n’est **pas** un revenu garanti court terme : une série de wins peut arriver. Dimensionne le bankroll (recommandé : ≥ 30–50× la mise max).

## Légal

Jeu d’argent. À vérifier au Québec / Canada (Loto-Québec, lois provinciales) avant tout mainnet public. Ce repo = code + tests, pas une licence d’opérateur.

**Pas audité. Devnet d’abord.**

## Build

```bash
git clone https://github.com/luxaway/pridevault-casino
cd pridevault-casino
cargo install multiversx-sc-meta --locked
sc-meta all build
```

## Endpoints clés

| Endpoint | Qui | Rôle |
|---|---|---|
| `init(treasury, min_bet, max_bet, round_blocks)` | deploy | setup |
| `fundBankroll` | owner, payable EGLD | remplir la caisse |
| `startRound` | owner / auto | ouvrir un round |
| `placeBet(under)` | joueur, payable EGLD | miser |
| `resolveRound` | n’importe qui après deadline | tirer le seed + scorer |
| `claim` | gagnant | retirer |
| `skimToTreasury` | owner | envoyer le profit |
| `pause` / `unpause` | owner | stop urgence |
