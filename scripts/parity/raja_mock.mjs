#!/usr/bin/env node
// Deterministic external RajaOngkir stub for end-to-end Nest/Axum HTTP parity.
// Do not use real credentials or depend on the public provider in CI.
import { createServer } from 'node:http';

const couriers = 'jne:sicepat:ide:sap:jnt:ninja:tiki:lion:anteraja:pos:ncs:rex:rpx:sentral:star:wahana:dse';

createServer(async (req, res) => {
  const chunks = [];
  for await (const part of req) chunks.push(part);
  const form = new URLSearchParams(Buffer.concat(chunks).toString());
  const valid = req.method === 'POST'
    && req.url === '/calculate/domestic-cost'
    && req.headers.key === 'parity-raja-key'
    && req.headers['content-type']?.includes('application/x-www-form-urlencoded')
    && form.get('origin') === '1'
    && form.get('destination') === '2'
    && form.get('weight') === '1000'
    && form.get('courier') === couriers
    && !form.has('unrecognized');
  res.setHeader('Content-Type', 'application/json');
  res.statusCode = valid ? 200 : 422;
  res.end(JSON.stringify(valid
    ? {meta:{code:200,message:'Provider quote available'},data:[{courier:'jne',service:'REG',cost:12000}]}
    : {meta:{code:422,message:'Invalid quote request'},data:[]}));
}).listen(3199, '127.0.0.1', () => {
  console.log('RajaOngkir mock listening on 127.0.0.1:3199');
});
