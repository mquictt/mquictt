#!/bin/bash
set -ex
cd $(dirname $0)

rm -rf cert* rootca* client server

openssl req -new -x509 -batch -nodes -days 10000 -keyout rootca.key -out rootca.crt
openssl req -new -batch -nodes -sha256 -keyout cert.key -out cert.csr -subj '/C=IN/CN=localhost'
openssl x509 -req -days 10000 -in cert.csr -CA rootca.crt -CAkey rootca.key -CAcreateserial -out cert.crt
openssl verify -CAfile rootca.crt cert.crt

mkdir client/
mv cert.crt client/
mv cert.key client/

openssl req -new -batch -nodes -sha256 -keyout cert.key -out cert.csr -subj '/C=IN/CN=a'
openssl x509 -req -days 10000 -in cert.csr -CA rootca.crt -CAkey rootca.key -CAcreateserial -out cert.crt
openssl verify -CAfile rootca.crt cert.crt

mkdir server/
mv cert.crt server/
mv cert.key server/

rm cert.csr
rm rootca.key
rm rootca.srl
