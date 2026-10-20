import { createFixture, removeFixture } from './fixture'

export default function setup() {
  createFixture()
  return removeFixture
}
