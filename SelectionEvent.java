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
    protected SelectionResource sr;
    
    /** Creates a new instance of SelectionEvent */
    public SelectionEvent(Object source, SelectionResource sr) {
        super(source);
        this.sr = sr;
    }
    
    public TransparentData getTransparentData() {
        return sr.getTransparentData();
    }
    
    public SelectionResource getSelectionResource() {
        return sr;
    }
    
}
