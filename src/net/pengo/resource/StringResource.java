/*
 * Created on Jan 16, 2004
 */
package net.pengo.resource;

import java.util.List;

import net.pengo.propertyEditor.StringPage;

/**
 * @author Smiley
 */
public abstract class StringResource extends Resource {

    public StringResource() {
        super();
        // TODO Auto-generated constructor stub
    }
    
    public abstract String getValue();
    public abstract void setValue(String val);
    
    public Resource[] getSources() {
        return null;
    }
    
    public String valueDesc() {
        return getValue();
    }
    
    /** @return list of PropertyPage's */ 
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
        pp.add(new StringPage(this));
        return pp;
    }
    
    

}
