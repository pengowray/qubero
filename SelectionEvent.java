/*
 * SelectionEvent.java
 *
 * Created on 16 August 2002, 17:22
 */

/**
 *
 * @author  administrator
 */
class SelectionEvent extends java.util.EventObject {
    protected RawDataSelection rds;
    
    /** Creates a new instance of SelectionEvent */
    public SelectionEvent(Object source, RawDataSelection rds) {
        super(source);
        this.rds = rds;
    }
    
    public RawDataSelection getRawDataSelection() {
        return rds;
    }
    
}
