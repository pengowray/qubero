/*
 * CursorData.java
 *
 * Created on 4 September 2002, 11:40
 */

/**
 *
 * @author  administrator
 */
public class Cursor implements Comparable {
    long startBefore;
    
    /** Creates a new instance of CursorData */
    public Cursor(long startBefore) {
        this.startBefore = startBefore;
    }
    
    /** Compares this object with the specified object for order.  Returns a
     * negative integer, zero, or a positive integer as this object is less
     * than, equal to, or greater than the specified object.<p>
     *
     * @param   o the Object to be compared.
     * @return  a negative integer, zero, or a positive integer as this object
     * 		is less than, equal to, or greater than the specified object.
     *
     * @throws ClassCastException if the specified object's type prevents it
     *         from being compared to this Object.
     *
     *
     */
    public int compareTo(Cursor o) {
        long thisVal = startBefore;
        long anotherVal = o.startBefore;
        return (thisVal<anotherVal ? -1 : (thisVal==anotherVal ? 0 : 1));
    }
    
    public int compareTo(Data o) {
        long thisVal = startBefore;
        long anotherVal = o.getStart();
        return (thisVal <= anotherVal ? -1 : 1);
    }

    
    public int compareTo(Object o) {
        if (o instanceof Cursor) {
            return compareTo((Cursor)o);
        }
        if (o instanceof Data) {
            return compareTo((Data)o);
        }
        System.out.println("comparing a cursor with a..." + o.getClass() + " argh! " + o);
        return compareTo((Data)o);
    }
    
    
    public long getStartBefore() {
        return startBefore;
    }
    
    public String toString() {
        return "Cursor sits before: " + startBefore;
    }
    public boolean equals(Data obj) {
        return false;
    }    
    public boolean equals(Cursor obj) {
        return (obj.startBefore == this.startBefore);
    }    
    
}
