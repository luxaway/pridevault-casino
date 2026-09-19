# Audit interne — ROAR Dice v1 (pas un audit professionnel)

Contrat : `src/lib.rs`  
Stack : multiversx-sc 0.66.2  
Date : 2026-09-19

## Verdict

**Devnet seulement.** Ne pas mettre de bankroll mainnet sans audit tiers.

Le design de rounds (mises lockées → RNG au resolve) suit la doc MultiversX. Un roll dans la même tx que la mise serait tricheable.

## Ce qui tient

- Rake séparé : ROAR 2%, EGLD 4%
- Payout dans le même token que la mise (pas de mix de caisses)
- Liability réservée avant d’accepter une mise
- Mise max = 2% du bankroll libre + cap dur
- Claim séparé (pas de payout de masse au resolve)
- Skim bloqué tant que le buffer 2× min_bankroll n’est pas plein
- Pause owner + pause auto si les deux caisses sont minces

## Issues (sévérité)

### Haute
- `setRoarToken` owner peut changer le ticker ROAR pendant qu’il reste des claims — lock le changer si `total_claimable_roar > 0`.
- Division entière `amount * 100 / under * 9500 / 10000` tronque contre le joueur (ok pour la maison, à documenter).
- Seed du round = `RandomnessSource` au premier `resolveRound`. Un resolver ne peut pas choisir *quelles* mises gagner, mais le moment du premier resolve fixe le seed. Acceptable v1, pas un VRF.

### Moyenne
- `startRound` ne vérifie plus un floor de bankroll (remettre le require).
- Pause auto seulement si **EGLD et ROAR** sont minces — une caisse peut être vide et l’autre ouverte.
- `VecMapper::clear` coûte du gas si beaucoup de mises ; v1 : limiter les mises / round.
- Skim math (`extra/2 + accrued`) est opaque. Simplifier en v1.1.
- Pas de cap dur sur le nombre de mises par round (DoS gas au resolve, mitgé par pages de 50).

### Basse
- Events claim/skim ne distinguent pas EGLD vs ROAR.
- Owner `unpause` sans recheck bankroll.
- Aucun test d’intégration SpaceCraft écrit pour le dual-token.

## Hors scope

Loi Québec / Canada, KYC, licence d’opérateur. UI PrideVault désactive le bouton tant que `ADDRESSES.roarDice` est vide.
