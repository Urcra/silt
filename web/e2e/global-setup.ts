import { abandonFixtures, createFixture, removeFixture } from './fixture'

export default function setup() {
  abandonFixtures()
  createFixture()
  return () => {
    removeFixture()
    abandonFixtures()
  }
}
