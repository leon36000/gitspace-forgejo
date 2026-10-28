# CONTINUATION-219 — refus d’une PR étrangère sous le profil d’organisation

Exécution locale du 27 octobre 2026 dans le worktree `webhook-005-workspace`, déjà fortement modifié; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, nettoyage, commit, publication, changement de production, nouvelle dépendance, service externe ou appel de modèle.

## Question et dispositif

La preuve live CONTINUATION-204-A2 établit déjà la lecture d’une PR réelle d’organisation par le profil de réconciliation dédié; CONTINUATION-205 essaie une route de commentaire avec son credential read-only. Ce test couvre un cas de routage distinct : une livraison de PR signée pour un dépôt hors du tuple compilé `(hôte, propriétaire, dépôt)` doit être refusée avant la consommation du credential ou tout appel amont.

Le test démarre le vrai receiver webhook Rust, lui envoie un événement `pull_request` synthétique signé pour `outside-org/unconfigured-repository`, puis arrête le receiver et lance le vrai exécutable `forgejo-reconcile-local` avec `--reconcile-target-profile org-team-review-e2e`. Un credential synthétique passe par un pipe. Une fixture HTTP authentifiée écoute l’endpoint loopback fixe du profil `127.0.0.1:3130` et compte les requêtes. Ce serveur HTTP n’est pas Forgejo.

## Résultat observé

Le receiver accepte l’événement signé (HTTP 202) et persiste une livraison. Le CLI termine avec code 1 et renvoie le schéma single v10, verdict `inconclusive`, erreur `resource_out_of_scope`, `upstream_read_attempts=0` et aucun `canonical_state`. La fixture HTTP ne reçoit aucune requête. Tous les octets du credential synthétique restent dans le pipe; le token n’apparaît ni dans stdout ni dans stderr. Les octets de l’inbox après réconciliation sont identiques au snapshot pris avant le CLI. Il s’agit d’un test processus ciblé (`n=1`), ignoré par défaut pour réserver le port 3130, puis exécuté explicitement. Sa sortie brute réussie est conservée dans `focused-test.log`.

La cible complète `webhook_receiver_process_e2e` se termine aussi avec 35 tests réussis, 0 échec et 103 ignorés. Les tests ignorés ne sont donc pas couverts par ce résultat; le nouveau scénario a été exécuté séparément.

Le premier essai ciblé a révélé une assertion partagée de test restée aux schémas single/batch v9/v10, alors que le CLI émet v10/v11. Le helper a été corrigé pour v10/v11. La sortie brute de cet échec initial n’a pas été conservée; le run exact réussi ultérieur est enregistré dans `focused-test.log`. Aucune source de l’implémentation de réconciliation n’a changé dans ce lot.

## Vérification et reproduction

Les commandes sont listées dans `commands.txt`. `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le garde de métadonnées rapporte `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION` et `status=metadata_policy_verified`.

La régression est dans `tests/webhook_receiver_process_e2e.rs`; le helper partagé y attend maintenant les schémas actuels du CLI. Aucun fichier Rust de production ni code Forgejo n’a été modifié dans ce lot. Le test requiert le port local fixe et ne doit pas être parallélisé avec un autre test qui réserve ce port.

## Limites

Il s’agit d’une seule livraison signée synthétique et d’un profil compilé, avec une fixture HTTP loopback plutôt qu’un serveur Forgejo. Le test établit ce refus de chemin avant lecture API/credential pour ce profil; il ne constitue pas une frontière réseau imposée au processus, un audit exhaustif des routes ou la preuve qu’un processus agent ne peut atteindre Forgejo par un autre chemin. Aucun modèle, AgentOS, autorité de fusion, comportement post-fusion ou comparaison contrôlée n’est exercé. Les 103 tests ignorés de la cible restent hors de ce run. Le projet cumulatif demeure incomplet.
