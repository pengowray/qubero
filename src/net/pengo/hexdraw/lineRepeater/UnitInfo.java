/**
 * UnitInfo.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.hexdraw.lineRepeater;

//FIXME: make flyweight
class UnitInfo {
    static final int NORMAL = 0;
    static final int EMPTY = 1;
    static final int SELECTED = 2;
    static final int SELECTED_NOFOCUS = 3;
    static final int EDITING = 4;
    static final int EDITING_NOFOCUS = 5; // another view of this byte is being edited

    protected byte b;
    protected int state;
    
    public UnitInfo(byte b, int state) {
        this.b = b;
        this.state = state;
    }
    public byte getByte(){
        return b;
    }
    public int getState() {
        return state;
    }

    public String hexString() {
        //FIXME: cache this / lazy init / table of 256 strings?
        
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return ""
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}


