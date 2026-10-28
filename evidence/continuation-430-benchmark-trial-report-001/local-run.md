# CONTINUATION-430 — préflight des reçus de comparaison — 28 octobre 2026

## Changement

`scripts/benchmark_trial_report.py` ajoute un lecteur JSONL versionné, une validation des paires et un calcul descriptif pour le protocole déjà défini dans `docs/architecture.md`. Le rapport agrège critères satisfaits, régressions échappées, interventions, durée, tokens, coût et livraison vérifiée; pour les métriques appariées, il rapporte médiane et écart absolu médian (MAD). Il conserve chaque essai, y compris non fusionné ou échoué. Il interdit toute conclusion automatisée de gain (`gain_claim_supported=false`).

Le schéma refuse les champs manquants/inattendus, les bras absents/dupliqués, le même ordre dans les bras d’une paire, une paire sans contrôles partagés identiques, des changements de modèle/budget dans une campagne, un runtime qui varie au sein d’un bras, un couple tâche/répétition réutilisé sous un autre ID, ou des versions différentes du commit/demande/oracle entre répétitions d’une même tâche. `pre_merge` relie son verdict au candidat et à la base; une fusion requiert un `post_merge` lié au commit fusionné. Un résultat post-fusion échec/inconclusif reste compté comme livraison non vérifiée; la dérive de base et les fusions faites malgré un échec pré-fusion sont signalées. Le label de vérificateur doit différer du label de runtime, sans authentifier ces identités. Le coût doit être présent avec sa source et une référence/hash de preuve; les références absolues, traversantes ou non portables sont refusées. Les hash ne sont que vérifiés au format.

Le statut de calibration de 36 paires exige exactement 12 tâches avec trois répétitions chacune, en plus d’un ordre équilibré. Ces 36 paires restent `complete_not_confirmatory` : ce compte est un plan de calibration, pas un effectif statistique confirmatoire.

## Vérifications

- Le test initial rouge a révélé qu’un ensemble de 36 paires en forme 9×4 était affiché complet, qu’un même couple tâche/répétition pouvait être dupliqué sous un autre `pair_id`, et qu’un chemin `..\\` Windows passait le contrôle POSIX. L’évolution vers le binding commit/base/vérificateur a ajouté une phase rouge où les nouveaux champs stricts échouaient, puis des refus ciblés pour candidat, base, commit fusionné et identité déclarée du vérificateur.
- Après correction, `tests/test_benchmark_trial_report.py` : **25/25 réussis**.
- `tests/test_latest_sha256_manifest.py` : **5/5 réussis**.
- `py_compile` des deux fichiers Python : code 0.
- `git diff --check` : code 0.
- Le vérificateur latest-valid du manifeste global et `sha256sum -c` du manifeste local passent après append; aucun chemin absent, non sûr ou divergent.

Commandes, versions et empreintes source sont dans les fichiers voisins. Le checkout était déjà fortement modifié; aucun état préalable n’a été réinitialisé. Le test de rapport CLI utilise des reçus synthétiques temporaires.

## Limites

Aucun benchmark réel, aucune tâche logicielle mesurée et aucun appel LLM n’ont été exécutés. Les coûts et tokens de comparaison sont non mesurés, pas imputés à zéro. Le calculateur n’ouvre pas les chemins référencés, ne vérifie ni leur empreinte réelle, ni la source de facturation, ni l’identité du vérificateur, ni l’authenticité des reçus. Le résultat ne démontre aucun effet causal ou gain. Un corpus de tâches autorisé, un oracle indépendant, un runtime agent disponible et un budget contrôlable restent nécessaires.

