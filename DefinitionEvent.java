/*
 * ClipboardEvent.java
 *
 * Created on 16 August 2002, 18:17
 */

/**
 *
 * @author  administrator
 */
class DefinitionEvent extends java.util.EventObject {
    protected RawDataSelection rds;
    
    public DefinitionEvent(Object source, RawDataSelection rds) {
        super(source);
        this.rds = rds;
    }
    
    public RawDataSelection getRawDataSelection() {
        return rds;
    }
}
