# CONTINUATION-350 — E2E processus des assignees et vérification locale

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les changements de code de ce lot sont limités au test processus et à sa liste d’autorisation; aucun comportement de production n’a été modifié. Aucun commit ni transfert GitLab n’a été effectué.

## Test processus ajouté

`tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_projects_current_issue_assignees_from_api` admet trois webhooks `issues/edited` signés par le receiver, rejoue le journal dans un vrai processus `forgejo-reconcile-local` et sert les GET depuis un fixture HTTP borné à `127.0.0.1:3130`. Le processus est exécuté par `scripts/run_webhook_reconcile_cli_e2e.py` dans Bubblewrap avec l’espace réseau externe désactivé.

Les réponses fixture fournissent, respectivement :

1. deux assignees non triés; le CLI les retourne triés par ID;
2. `assignees: null`; le CLI conserve l’inconnu (`null`);
3. `assignees: []`; le CLI retourne un tableau vide connu avec les sources/fraîcheur/trust attendus.

Les webhooks contiennent à la place des identités périmées. Le résultat canonique n’en expose aucune. Le test exige trois routes `/api/v1/repos/forge-admin/agent-forge-core/issues/{41,42,43}`, aucune requête Authorization et un journal identique octet pour octet après la relecture. Résultat : 1 test réussi, 0 échec; 169 autres tests du binaire sont filtrés.

Commande reproductible :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_projects_current_issue_assignees_from_api
```

Les premières exécutions ont corrigé des défauts d’assertion du nouveau test et découvert un garde du harnais périmé : le binaire retourne le schéma batch 21 tandis que le helper s’attendait à 20. Le helper attend maintenant les versions single/batch 17/21. Le code CLI était déjà à ces versions; aucune modification de logique de production n’a été nécessaire.

## Vérifications

Environnement : Cargo 1.95.0, rustc 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux aarch64. La sortie complète de la suite Cargo se trouve dans [`cargo-all-targets.log`](cargo-all-targets.log).

| Commande | Résultat |
| --- | --- |
| `cargo test --offline --locked --all-targets -- --test-threads=1` | Code 0; 38 groupes, 735 réussis, 0 échec, 162 ignorés. Le test processus ajouté est ignoré par Cargo et lancé séparément par Bubblewrap ci-dessus. |
| `cargo fmt --all -- --check` | Code 0 après formatage de la nouvelle assertion. |
| `git diff --check` | Code 0. |
| `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` | Code 0. |
| `python3 scripts/verify_cargo_license_state.py` | Code 0; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`. |
| `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v scripts/test_cargo_license_state.py` | 12 réussis, 0 échec. |

Le test ajouté utilise un adaptateur HTTP local déterministe; il ne constitue pas un test live Forgejo. Il n’utilise ni AgentOS ni un LLM. Le résultat prouve une couture receiver → journal → processus CLI → readback fixture seulement, pas la qualité générale du produit.

## Transfert GitLab PC4

À la demande de l’utilisateur, le courriel d’accès PC4 a été consulté. Il contient un compte root et un mot de passe enregistré; le secret n’est pas reproduit ici, passé à une commande, conservé dans les preuves ou utilisé. Le jeton trouvé dans un autre courriel cible des projets Nexora et n’est pas adapté au dépôt GitLab demandé.

Sondes sans authentification : `curl` vers `https://pc4.tail0c55ff.ts.net/users/sign_in` retourne le code 6 (échec DNS); un essai HTTPS dirigé vers l’adresse Tailscale du pair PC4 retourne le code 28 (connexion TCP 443 expirée après 3 s). Le remote du worktree demeure la Forgejo locale de test `http://127.0.0.1:3130/forge-admin/agent-forge-core.git`. Aucun `git ls-remote`, login, lecture des refs du GitLab, commit ou push n’a été réalisé.

L’utilisateur a autorisé le transfert, mais PC4 n’est pas joignable depuis PC6; aucune donnée du dépôt n’a donc été envoyée. Prochaine action : rétablir la résolution MagicDNS ou une route autorisée vers HTTPS PC4; ensuite inspecter les refs distantes en lecture seule, auditer le contenu local pour les secrets/données privées, puis préparer une branche de migration dédiée sans écraser de refs existantes.

## État de complétion

Ce lot ne démontre pas le parcours LLM/AgentOS, un pull request/merge ou la validation post-fusion. Il ne change aucun résultat comparatif, coût, temps, interventions humaines ni gain. Les critères cumulés de la mission restent ouverts.
