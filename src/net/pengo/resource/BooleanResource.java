/*
 * BooleanResource.java
 *
 * Created on August 16, 2003, 10:50 PM
 */

package net.pengo.resource;

import java.util.List;

import net.pengo.propertyEditor.SetBooleanPage;

/**
 *
 * @author  Smiley
 */
abstract public class BooleanResource extends DefinitionResource {

    public BooleanResource() {
    }

    abstract public boolean getValue();
    abstract public void setValue(boolean b);

    abstract public boolean isPrimative();
    
    /*
    public void editProperties() {
        new BooleanPrimativeResourcePropertiesForm(BooleanResource.this).show();
    }
    */
    
    public String valueDesc() {
        return getValue()+"";
    }    

    /** @return list of PropertyPage's */ 
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
        pp.add(new SetBooleanPage(this));
        return pp;
    }    
}
