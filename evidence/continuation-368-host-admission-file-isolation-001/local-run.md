# CONTINUATION-368 — canari d’admission hôte invisible au processus Bubblewrap

Date : 28 octobre 2026. Checkout de test : worktree local `webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’environnement et les empreintes des sources sont consignés dans [environment.txt](environment.txt) et [source-sha256.txt](source-sha256.txt).

## Question vérifiée

La sonde processus Bubblewrap détectait déjà l’absence d’un token Forgejo synthétique, l’isolation réseau et l’absence des routes API, Web et receive-pack. Cette tranche ajoute une vérification manquante : l’enfant ne peut pas lire le fichier privé `host-admission.json` qui représente le handoff d’admission hôte.

## Fixture et changement

`tests/test_mission_012_sandboxed_test_adapter.py::SandboxedAdapterTests::test_probe_process_remains_isolated_until_reprobe_and_release` crée un canari JSON synthétique, sous `/tmp/.runtime/host-admission.json`, avec répertoire privé et fichier mode 0600. Le chemin est ajouté à `private_file_probes` avec le label existant `host_admission`. Le résultat attendu énumère ce label dans `private_runtime_files_not_visible`; après la libération du vrai processus enfant, le test vérifie aussi que les octets du canari hôte n’ont pas changé. Aucun secret ou credential réel n’est utilisé. Le probe se fait dans le processus Bubblewrap et ne modifie pas le montage produit.

L’essai test-first a d’abord échoué sur l’assertion attendue de labels, car le canari n’avait pas encore été raccordé au probe. Ce n’était pas un échec de sécurité du sandbox. L’observation est décrite dans [test-first-observation.md](test-first-observation.md); sa sortie brute complète n’a pas été archivée. Après raccordement, le test ciblé passe.

## Vérification

Commande finale :

```text
python3 -m unittest discover -s tests -p 'test_mission_012_sandboxed_test_adapter.py' -v
```

Résultat : **17 tests réussis, 0 échec** en 0,787 s. Le log complet est [python-module-tests.log](python-module-tests.log). Le scénario du canari passe au sein d’un processus Bubblewrap réel et vérifie notamment `host_admission` invisible, l’absence du credential canari, les trois routes réseau refusées, les FDs `[0, 1, 2]`, la re-sonde puis la libération du processus. `git diff --check` n’a signalé aucun problème sur les changements suivis; la nouvelle fixture de test Python a également été exécutée par le module complet.

Cette tranche est une **extension de test**; elle ne corrige pas une fuite démontrée et ne change pas l’implémentation du sandbox. Elle teste une configuration synthétique, une exécution locale (`n=1`) et l’invisibilité du chemin. Il n’y a pas de sonde d’écriture dédiée sur `host-admission.json`; l’assertion d’octets inchangés n’est pas une preuve qu’une tentative d’écriture a été faite. Aucun processus AgentOS/LLM, route merge, commande de vérification protégée, origine de digest, acteur privilégié ou évasion noyau n’a été évalué. La provenance de la politique hôte reste ouverte.

## Transfert GitLab PC4

À la demande de transfert de l’utilisateur, le remote `pc4-gitlab` a été ajouté à la configuration locale sans credential dans l’URL; `origin` vers Forgejo local a été conservé. Les contrôles non authentifiés consignés dans [pc4-connectivity.txt](pc4-connectivity.txt) montrent que PC4 est en ligne dans Tailscale, tandis que son nom MagicDNS ne résout pas et que TCP 22/443 est inaccessible depuis ce poste. Aucun fetch, lecture de refs, commit de migration ou push n’a eu lieu. Le PAT déjà signalé comme exposé et visant un autre projet n’a pas été réutilisé. Un accès de projet réémis et une route PC4 fonctionnelle sont nécessaires pour reprendre le transfert.
