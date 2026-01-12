#!/bin/bash
if ! (return 0 2>/dev/null); then echo "Executed"; else echo "Sourced"; fi
