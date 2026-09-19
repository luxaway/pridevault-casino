# PrideVault Casino — ROAR Dice (self-sustaining)

Repo: https://github.com/luxaway/pridevault-casino

Le jeu doit vivre sur **ses mises**, pas sur des recharges owner toutes les semaines.

## Modèle éco

1. **Seed unique** — tu fonds le bankroll une fois (`fundBankroll`).
2. **Rake 2%** sur chaque mise — revenu certain, resté dans le contrat jusqu’à ce que le buffer soit plein.
3. **Edge 5%** sur les payouts — avantage maison long terme.
4. **Mise max dynamique** = 2% du bankroll libre, plafonnée par `cap_max_bet`. Une win streak ne peut pas vider la caisse.
5. **Pause auto** si le bankroll libre passe sous `min_bankroll`.
6. **Skim** seulement au-dessus de **2× min_bankroll**, et seulement la moitié du surplus + rake. Le reste reste pour jouer.

Après le seed, PrideVault encaisse via `skimToTreasury` quand ça roule. Pas besoin de réinjecter — si ça pause, tu attends le volume ou tu réduis encore les mises (le cap dynamique le fait tout seul).

## Règles de jeu

- Dés 0–99, `under` 2–96, win si `roll < under`
- Payout sur le **net** (mise − rake 2%), avec edge 5%
- Ex. 1 EGLD @ under 50 → net 0.98 → payout win ≈ 1.86

## Seed recommandé (devnet / petit lancement)

| | Exemple |
|---|---|
| Seed bankroll | 10 EGLD |
| min_bankroll | 5 EGLD |
| min_bet | 0.05 EGLD |
| cap_max_bet | 0.20 EGLD |
| Mise max réelle au départ | min(0.20, 2% de ~10) = 0.20 cap |

Monte le cap seulement quand le bankroll a grossi.

**Pas audité. Devnet d’abord. Jeu d’argent : vérifier le cadre Québec/Canada avant un mainnet public.**
