# CONTINUATION-359 — tentative d’accès GitLab PC4 après installation

Date : 28 octobre 2026. Cible communiquée : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`. Cette tranche ne change pas le code du produit; elle documente une vérification réseau de lecture seule et le point d’arrêt du transfert.

## Vérification observée

Après que l’utilisateur a indiqué avoir installé GitLab sur PC4, une vérification non authentifiée a été faite depuis PC6 :

- `getent ahosts pc4.tail0c55ff.ts.net` n’a retourné aucune adresse (`dns=unresolved`).
- La requête `curl --disable --noproxy '*' --connect-timeout 3 --max-time 8 --silent --show-error --output /dev/null --write-out ... https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` a échoué sur la résolution du nom (`Could not resolve host`, code curl 6, HTTP 000).
- `env -i PATH=/usr/bin:/bin GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 git -c credential.helper= -c credential.interactive=never ls-remote https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` a échoué à la résolution (code 128). Les helpers et prompts Git étaient désactivés; aucune référence distante n’a été lue.
- `origin` demeure `http://127.0.0.1:3130/forge-admin/agent-forge-core.git`.

Les sorties brutes de ces commandes n’ont pas été conservées dans un fichier de log durable; les faits ci-dessus sont le relevé de la sortie observée. Le test conclut seulement que le FQDN n’était pas résolu depuis PC6 à ce moment. Il n’établit pas que GitLab est arrêté ou mal installé sur PC4.

## État du dépôt et sécurité

HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Juste avant cette preuve, 19 chemins suivis étaient modifiés et 13 900 chemins non suivis. Aucun fichier n’a été supprimé, aucun contenu n’a été stagé ou commité, aucun login, changement de credential ou push n’a eu lieu. Le comptage est l’état observé avant l’ajout de ce reçu et des deux mises à jour documentaires.

Aucun email n’a été relu et aucun secret utilisé. CONT-357 signale que le PAT reçu par courriel a déjà figuré dans une sortie du connecteur; ce jeton doit être révoqué/renouvelé et ne sera pas réutilisé. Ne pas utiliser le mot de passe root initial. Utiliser ensuite un credential dédié au projet et à portée minimale, injecté localement sans l’écrire dans le dépôt.

## Suite

Attendre un changement d’état DNS/réseau ou une information vérifiable de PC4; ne pas répéter cette sonde en boucle. Continuer les travaux locaux distincts. Quand PC4 est joignable : vérifier les références distantes en lecture seule, auditer les fichiers exportables et les chemins non suivis, préparer une branche `codex/`, puis seulement transférer le contenu autorisé sans toucher à `main`. Aucun push n’est effectué par ce lot.

