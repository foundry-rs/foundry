/* global describe it */

const ledger = require('@summa-tx/coins-ledger')

describe('coins_ledger', () => {
  it('debug_send', async () => {
    const device = await ledger.LedgerTransport.create()
    const resp = await device.debug_send()
    console.log(resp)
  })
})
