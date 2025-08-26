class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.7.2"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.2/scriba-x86_64-apple-darwin"
    sha256 "90ba8bdc130bb7eb669bfb7d043d04d56537043d3419baee0801c46fcde05b22"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.2/scriba-aarch64-apple-darwin"
    sha256 "167758888db4be7aebc6eb8522c99867f07eefe3d82d118a76de365df4253b25"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
