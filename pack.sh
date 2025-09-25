#!/bin/bash

bn=$(basename $(readlink -f .))

(echo ".git"; git ls-files --cached --others --exclude-standard) | 
    awk -v bn="$bn" '{ print bn"/"$0 }' |
    tar -C .. -czvf "$bn.tar.gz" --files-from=-
