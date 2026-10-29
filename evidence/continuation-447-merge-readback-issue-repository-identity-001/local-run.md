# CONTINUATION-447 — lier l’issue au dépôt attendu dans le readback de fusion

Date : 28 octobre 2026. Worktree : `.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà très modifié et contenait de nombreux fichiers non suivis. Aucun reset, nettoyage, commit, push, accès distant ou appel de modèle n’a été effectué.

## Lacune et changement

`merge_readback_is_confirmed` liait déjà le readback PR aux commits, à la branche cible et à l’identité complète de `base.repo`, mais acceptait un objet issue dès que son numéro correspondait à la PR et que son état valait `closed`. Il ne vérifiait pas `issue.repository`. Un objet d’issue d’un autre dépôt pouvait donc participer à une conclusion positive.

Forgejo v17.0.0 confirme que l’API `GET /issues/{index}` expose bien cette identité : `Issue.Repo *RepositoryMeta`, `RepositoryMeta.owner` chaîne, `name` et `full_name`; le convertisseur les remplit et la route `GetIssue` utilise ce convertisseur. Les références exactes, le commit et les empreintes figurent dans `upstream-source.txt`.

Le readback exige maintenant l’objet `repository` et les trois valeurs égales à l’owner/dépôt fixe du profil sélectionné. Une valeur manquante, mal typée ou différente fait échouer `merge_readback_is_confirmed`, ce qui garde le résultat `inconclusive`. Le contrat est annoncé dans `--capabilities.postmerge_issue_identity`. Cela lie les champs de la réponse au profil; cela n’authentifie pas le serveur.

## Preuves observées

Le test unitaire existant `merge_readback_refuses_divergent_commit_and_issue_state` a été enrichi. La version rouge a échoué précisément parce qu’un owner d’issue étranger était accepté (`red.log`). Après le changement, le même test passe 1/1 (`unit-focused.log`). Il refuse aussi les noms et full names étrangers, les trois champs manquants et l’objet `repository` absent. La suite du binaire passe 167/167 (`bin-tests.log`).

Le test processus `merged_readback_with_contradictory_issue_repository_is_inconclusive` utilise le vrai CLI sous Bubblewrap avec réseau et PID isolés, et une fixture HTTP loopback. La fixture applique le POST de fusion puis répond au GET issue avec un owner différent. Le CLI renvoie `inconclusive / merge_outcome_not_confirmed`; l’intention `mission_merge_attempt_started_v1` existe et aucune résolution `mission_merge_attempt_resolved_v1` n’est écrite. Le scénario passe 1/1 (`process-e2e.log`). Le processus a réellement effectué le POST sur la fixture; le contrôle empêche l’annonce d’une confirmation client, pas l’effet serveur déjà appliqué.

Sur ce snapshot courant, le run all-targets passe 39 groupes, 786 réussis, 0 échec, 208 ignorés (`all-targets.log`). Un premier run a échoué à cause de fixtures de profil qui n’incluaient pas encore le champ repository requis; la correction a ajouté l’identité attendue à ces fixtures et n’a pas affaibli le prédicat. Le journal du run intermédiaire est conservé dans `all-targets-fixture-fail.log`. Les tests ignorés ne sont pas présentés comme exécutés. Le test processus de ce lot est ignoré par défaut et a été exécuté séparément.

Format Rust, vérification de diff, garde des métadonnées de licence, compilation du runner Python et existence du lien documentaire passent dans `final-checks.log`. Le fichier `pre-report-checks.log` conserve une première passe de contrôles lancée avant la création de ce rapport : son unique erreur est le lien vers le rapport qui n’existait pas encore; le contrôle a été relancé après création et passe dans `final-checks.log`. `environment.txt` consigne les outils et HEAD; `SHA256SUMS` couvre les logs et les références.

## Limites

La preuve E2E est un cas `n=1`, sur API loopback synthétique et non sur un Forgejo vivant. Elle ne prouve ni l’authenticité des réponses, ni une fusion atomique par rapport aux autres writers, ni une annulation/compensation après POST, ni tous les comportements serveur possibles. Les empreintes et signatures établissent l’intégrité/provenance des artefacts locaux, pas leur correction. Aucun AgentOS/LLM, benchmark comparatif, gain de productivité, revue juridique ou licence de redistribution n’est établi dans ce lot. Le statut licence Cargo demeure `no_license_selected`; droits `not_assessed`.

La mission globale reste incomplète. Prochaine action : prendre un risque non couvert distinct dans la matrice, poursuivre l’intégration locale et maintenir les opérations distantes et les appels de modèle conditionnés aux autorisations et limites déjà consignées.
