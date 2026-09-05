import test from 'node:test';
import assert from 'node:assert/strict';
import {load} from './ui_alias_harness.mjs';
const {strategyOrderCeiling}=load('lib/orderLimits');
test('order ceiling combines capital cap with explicit cap without turning missing data into unlimited',()=>{
 assert.deepEqual(strategyOrderCeiling(5,400,300),{known:true,ceiling:0.75});
 assert.deepEqual(strategyOrderCeiling(5,400,6000),{known:true,ceiling:5});
 assert.deepEqual(strategyOrderCeiling(0,400,600),{known:true,ceiling:1.5});
 assert.deepEqual(strategyOrderCeiling(0,0,0),{known:true,ceiling:null});
 for(const args of [[undefined,0,300],[5,undefined,300],[5,400,NaN],[5,400,0],[-1,0,300]])assert.deepEqual(strategyOrderCeiling(...args),{known:false,ceiling:null});
});
