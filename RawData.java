/** 
 * RawData is a fixed length chunk of data. e.g. a binary file, or an area of memory.
 */
abstract class RawData implements Chunk {
    abstract public byte[] getData();
    abstract public int getLength();
    
    //XXX: throw out of bounds thing
    public RawDataSelection getSelection(int start, int length) {
        return new RawDataSelection(this, start, length);
    }
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "raw data";
    }
        
    public String getType() {
        return "Raw data";
    }


}
