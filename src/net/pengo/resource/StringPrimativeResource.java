/*
 * StringPrimativeResource.java
 *
 * Created on 17 September 2004, 09:09
 */

package net.pengo.resource;

/**
 *
 * @author  Que
 */
public class StringPrimativeResource extends StringResource {
    private String value = "";
    
    /** Creates a new instance of StringPrimativeResource */
    public StringPrimativeResource() {
    }

    public StringPrimativeResource(String value) {
	setValue(value);
    }
    
    public String getValue() {
	return value;
    }
    
    public void setValue(String val) {
	this.value = val;
	alertChangeListenerer();
    }
    
}
