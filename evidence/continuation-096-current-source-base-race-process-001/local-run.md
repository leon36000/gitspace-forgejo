# CONTINUATION-096 — course de base retestée sur les sources locales courantes

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Aucun commit, déploiement, ou appel externe.

## Résultat observé

Commande :

```text
python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only external_base_advance_during_merge_post_is_fail_closed_but_still_mutates_branch
```

Le runner compile le test depuis le snapshot des sources du worktree, hors ligne et verrouillé, puis l’exécute sous Bubblewrap avec namespaces réseau et PID distincts, limites CPU/mémoire et source montée en lecture seule. Résultat : 1 test réussi, 0 échec, 7 ignorés. La preuve du run et la commande complète sont dans `focused-isolated-run.log`; le hash du snapshot source rapporté par le runner est `2201f21df9265b9c0e8ae1e33f0b9734086b3eb1d844f38eae107d70b96cee05`.

La barrière du fixture retarde l’application du POST `/pulls/23/merge`. Après la dernière lecture par le CLI, le fixture fait avancer `main` de `1111111111111111111111111111111111111111` à `4444444444444444444444444444444444444444`, puis accepte le POST. Le serveur simulé marque alors la cible fusionnée avec le nouveau parent. Le vrai CLI retourne un échec de processus et `inconclusive/merge_outcome_not_confirmed`, `merge_attempted=true`; la reprise relit l’état, ne renvoie pas de POST, et garde l’intention sans résolution terminale. Un seul POST est observé.

## Interprétation et limite

Ce retest actuel confirme la détection fail-closed et la reprise sans double POST. Il confirme aussi que le client ne peut pas empêcher la mutation sur une base que le gate n’a pas vérifiée : Forgejo v17 accepte une requête de merge sans compare-and-swap de la base. C’est un fixture déterministe, pas un test contre Forgejo live; il ne ferme pas la fenêtre TOCTOU. L’audit source Forgejo officiel épinglé à `0d791421f85ca13a63b665053d81ea765fbbc571` et ses tests de writers/LFS restent décrits dans [l’inventaire](../../docs/source-inventory.md) et [la décision d’architecture](../../docs/architecture.md). Aucun patch scratch n’est adopté.

La frontière agent demandée a déjà été éprouvée dans CONTINUATION-054 et recontrôlée sur les sources courantes par CONTINUATION-088 : le vrai child MCP Bubblewrap ne joint pas les routes nommées et ne reçoit pas le credential, tandis que le gate hôte peut lire et fusionner sur l’instance de test. Je n’ai pas rejoué ces scénarios. Cette séparation ne bloque pas un autre writer Forgejo autorisé ni un processus hôte du même UID.

Versions : rustc `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Python `3.12.3`, Bubblewrap `0.9.0`, Linux ARM64. Aucun LLM, AgentOS, coût fournisseur, secret réel, production ou instance Forgejo live n’a été utilisé. Projet incomplet.
