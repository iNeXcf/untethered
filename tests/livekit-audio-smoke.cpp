// Standalone smoke test for the LiveKit C++ SDK 1.13.1. Build with C++17,
// -pthread, the SDK include directory, and -llivekit. The runtime also
// needs liblivekit_ffi.so on its library path.
//
// Set LIVEKIT_URL, LIVEKIT_FIRST_TOKEN and LIVEKIT_SECOND_TOKEN. Use a
// dedicated test room and two distinct identities allowed to publish and
// subscribe. The test uses synthetic audio and in-process test keys.

#include <livekit/livekit.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <future>
#include <iostream>
#include <mutex>
#include <memory>
#include <random>
#include <stdexcept>
#include <thread>
#include <utility>
#include <vector>

using namespace std::chrono_literals;

namespace {
constexpr int SampleRate = 48000;
constexpr int FrameSamples = SampleRate / 100;

struct EncryptedTrack {
    std::shared_ptr<livekit::Track> track;
    bool encrypted;
};

class TrackReceiver final : public livekit::RoomDelegate {
public:
    std::future<EncryptedTrack> receive() { return subscribed.get_future(); }

    void onTrackSubscribed(livekit::Room &, const livekit::TrackSubscribedEvent &event) override {
        if (!event.track || !event.publication || event.track->kind() != livekit::TrackKind::KIND_AUDIO)
            return;
        std::lock_guard lock(mutex);
        if (captured) return;
        captured = true;
        subscribed.set_value({event.track, event.publication->encryptionType() == livekit::EncryptionType::GCM});
    }

private:
    std::mutex mutex;
    bool captured = false;
    std::promise<EncryptedTrack> subscribed;
};

const char *requiredEnvironment(const char *name) {
    const auto *value = std::getenv(name);
    if (!value || !*value) throw std::runtime_error(std::string("Set ") + name);
    return value;
}

std::vector<std::uint8_t> randomKey() {
    std::random_device random;
    std::uniform_int_distribution<unsigned> byte(0, 255);
    std::vector<std::uint8_t> key(32);
    for (auto &value : key) value = static_cast<std::uint8_t>(byte(random));
    return key;
}

void installKeys(livekit::Room &room, const std::string &firstIdentity, const std::string &secondIdentity,
                 const std::vector<std::uint8_t> &firstKey, const std::vector<std::uint8_t> &secondKey) {
    auto encryption = room.e2eeManager().lock();
    if (!encryption) throw std::runtime_error("LiveKit encryption manager unavailable");
    auto keys = encryption->keyProvider().lock();
    if (!keys) throw std::runtime_error("LiveKit key provider unavailable");
    keys->setKey(firstIdentity, firstKey);
    keys->setKey(secondIdentity, secondKey);
    encryption->setEnabled(true);
}

std::shared_ptr<livekit::AudioStream> openStream(std::future<EncryptedTrack> &received) {
    if (received.wait_for(10s) != std::future_status::ready)
        throw std::runtime_error("Timed out waiting for a remote audio track");
    auto event = received.get();
    if (!event.encrypted) throw std::runtime_error("Remote track is not encrypted");
    livekit::AudioStream::Options options;
    options.capacity = 32;
    return livekit::AudioStream::fromTrack(event.track, options);
}

livekit::AudioFrame tone(double frequency, int offset) {
    std::vector<std::int16_t> samples(FrameSamples);
    for (int i = 0; i < FrameSamples; ++i)
        samples[i] = static_cast<std::int16_t>(12000 * std::sin(6.28318530718 * frequency * (offset + i) / SampleRate));
    return {std::move(samples), SampleRate, 1, FrameSamples};
}

bool hasAudio(livekit::AudioStream &stream, const std::atomic_bool &stop) {
    livekit::AudioFrameEvent event;
    while (!stop && stream.read(event)) {
        const auto &samples = event.frame.data();
        const auto audible = std::count_if(samples.begin(), samples.end(), [](std::int16_t value) {
            return std::abs(static_cast<int>(value)) > 500;
        });
        if (audible > 30) return true;
    }
    return false;
}

void runSmokeTest() {
    const auto *url = requiredEnvironment("LIVEKIT_URL");
    const auto *firstToken = requiredEnvironment("LIVEKIT_FIRST_TOKEN");
    const auto *secondToken = requiredEnvironment("LIVEKIT_SECOND_TOKEN");
    TrackReceiver firstReceiver, secondReceiver;
    livekit::Room first, second;
    first.setDelegate(&firstReceiver);
    second.setDelegate(&secondReceiver);
    auto firstTrack = firstReceiver.receive();
    auto secondTrack = secondReceiver.receive();

    livekit::RoomOptions options;
    auto &keys = options.encryption.emplace().key_provider_options;
    keys.key_derivation_function = livekit::KeyDerivationFunction::HKDF;
    keys.ratchet_window_size = 10;
    keys.key_ring_size = 256;
    if (!first.connect(url, firstToken, options) || !second.connect(url, secondToken, options))
        throw std::runtime_error("Could not connect both test participants");

    const auto firstParticipant = first.localParticipant().lock();
    const auto secondParticipant = second.localParticipant().lock();
    if (!firstParticipant || !secondParticipant) throw std::runtime_error("Local participants unavailable");
    const auto firstIdentity = firstParticipant->identity();
    const auto secondIdentity = secondParticipant->identity();
    if (firstIdentity == secondIdentity) throw std::runtime_error("Use two distinct participant identities");
    const auto firstKey = randomKey();
    const auto secondKey = randomKey();
    installKeys(first, firstIdentity, secondIdentity, firstKey, secondKey);
    installKeys(second, firstIdentity, secondIdentity, firstKey, secondKey);

    auto firstSource = std::make_shared<livekit::AudioSource>(SampleRate, 1);
    auto secondSource = std::make_shared<livekit::AudioSource>(SampleRate, 1);
    firstParticipant->publishAudioTrack("test-tone-a", firstSource, livekit::TrackSource::SOURCE_MICROPHONE);
    secondParticipant->publishAudioTrack("test-tone-b", secondSource, livekit::TrackSource::SOURCE_MICROPHONE);
    auto heardByFirst = openStream(firstTrack);
    auto heardBySecond = openStream(secondTrack);

    std::atomic_bool stop = false;
    bool firstHeardAudio = false;
    bool secondHeardAudio = false;
    std::thread firstReader([&] { firstHeardAudio = hasAudio(*heardByFirst, stop); });
    std::thread secondReader;
    auto finish = [&] {
        stop = true;
        heardByFirst->close();
        heardBySecond->close();
        firstReader.join();
        if (secondReader.joinable()) secondReader.join();
    };
    try {
        secondReader = std::thread([&] { secondHeardAudio = hasAudio(*heardBySecond, stop); });
        auto deadline = std::chrono::steady_clock::now();
        for (int frame = 0; frame < 300; ++frame) {
            firstSource->captureFrame(tone(440, frame * FrameSamples));
            secondSource->captureFrame(tone(660, frame * FrameSamples));
            deadline += 10ms;
            std::this_thread::sleep_until(deadline);
        }
    } catch (...) {
        finish();
        throw;
    }
    finish();
    if (!firstHeardAudio || !secondHeardAudio)
        throw std::runtime_error("No decoded audio in one or both directions");
    const bool firstDisconnected = first.disconnect();
    const bool secondDisconnected = second.disconnect();
    if (!firstDisconnected || !secondDisconnected)
        throw std::runtime_error("Could not disconnect both test participants");
}
}

int main() {
    livekit::initialize(livekit::LogLevel::Warn);
    int status = 0;
    try {
        runSmokeTest();
        std::cout << "Encrypted native audio passed in both directions.\n";
    } catch (const std::exception &error) {
        std::cerr << "Native audio test failed: " << error.what() << '\n';
        status = 1;
    }
    livekit::shutdown();
    return status;
}
