/** 
 * NewFile.. currently immutable.
 */
class NewFileChunk extends RawData {
    static final byte[] nada = new byte[0];
    
    public byte[] getData() {
        return nada;
    };
    
    public int getLength() {
        return 0;
    }
    
    //XXX: throw out of bounds thing
    public RawDataSelection getSelection(int start, int length) {
        return new RawDataSelection(this, 0, 0);
    }
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "empty file";
    }
        
    public String getType() {
        return "empty file";
    }


}
