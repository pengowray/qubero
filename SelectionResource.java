/*
 * SelectionResource.java
 *
 * Created on 4 September 2002, 19:40
 */

/**
 *
 * @author  administrator
 */
public class SelectionResource extends Resource {
    final protected Data sel;
    
    public SelectionResource(OpenFile openFile, Data sel) {
	super(openFile);
        this.sel = sel;
    }
    
    public Data getData() {
        return sel;
    }
    
    public TransparentData getTransparentData() {
        if (sel instanceof TransparentData) {
            return (TransparentData)sel;
        }
        
        return sel.getTransparentData();
    }
    
    public boolean equals(SelectionResource o) {
        if (o == this)
            return true;
        
        return (sel.getStart() == o.sel.getStart() && sel.getLength() == o.sel.getLength());
    }
    
    public boolean equals(Object o) {
        if (o == this)
            return true;
        
        if (o instanceof SelectionResource) {
            return equals((SelectionResource)o);
        }
        
        return false;
    }
}
